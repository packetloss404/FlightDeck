use serde_json::Value;
use std::collections::VecDeque;

const RING_CAPACITY: usize = 5;
const LOOP_THRESHOLD: usize = 3;

#[derive(Debug, Clone, Default)]
pub struct DoomLoopDetector {
    recent: VecDeque<CallFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CallFingerprint {
    tool_name: String,
    canonical_input: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoomLoopVerdict {
    pub detected: bool,
    pub message: Option<String>,
}

impl DoomLoopDetector {
    pub fn new() -> Self {
        Self {
            recent: VecDeque::new(),
        }
    }

    /// Record a tool invocation and return whether we're in a doom loop.
    /// Threshold: the last 3 calls are identical (same tool + same canonical input).
    pub fn observe(&mut self, tool_name: &str, input: &Value) -> DoomLoopVerdict {
        let fp = CallFingerprint {
            tool_name: tool_name.to_string(),
            canonical_input: canonical(input),
        };

        self.recent.push_back(fp);
        while self.recent.len() > RING_CAPACITY {
            self.recent.pop_front();
        }

        if self.recent.len() >= LOOP_THRESHOLD {
            let tail_start = self.recent.len() - LOOP_THRESHOLD;
            let first = &self.recent[tail_start];
            let all_equal = (tail_start + 1..self.recent.len())
                .all(|i| &self.recent[i] == first);
            if all_equal {
                return DoomLoopVerdict {
                    detected: true,
                    message: Some(format!(
                        "same tool `{}` called 3× in a row with identical input — is the model stuck?",
                        tool_name
                    )),
                };
            }
        }

        DoomLoopVerdict {
            detected: false,
            message: None,
        }
    }

    /// Clear state — call after the user approves the "stuck" detection.
    pub fn reset(&mut self) {
        self.recent.clear();
    }
}

/// Serialize a JSON value with object keys sorted recursively.
fn canonical(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                // Use serde to escape the key safely.
                out.push_str(&serde_json::to_string(k).unwrap_or_else(|_| "\"\"".to_string()));
                out.push(':');
                write_canonical(&map[*k], out);
            }
            out.push('}');
        }
        Value::Array(arr) => {
            out.push('[');
            for (i, item) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => {
            out.push_str(&serde_json::to_string(other).unwrap_or_else(|_| "null".to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn three_identical_calls_detected_on_third() {
        let mut d = DoomLoopDetector::new();
        let input = json!({"path": "/tmp/x"});
        assert!(!d.observe("read", &input).detected);
        assert!(!d.observe("read", &input).detected);
        let v = d.observe("read", &input);
        assert!(v.detected);
        assert!(v.message.unwrap().contains("read"));
    }

    #[test]
    fn two_identical_then_different_not_detected() {
        let mut d = DoomLoopDetector::new();
        let a = json!({"path": "/tmp/x"});
        let b = json!({"path": "/tmp/y"});
        assert!(!d.observe("read", &a).detected);
        assert!(!d.observe("read", &a).detected);
        assert!(!d.observe("read", &b).detected);
    }

    #[test]
    fn same_tool_different_inputs_not_detected() {
        let mut d = DoomLoopDetector::new();
        assert!(!d.observe("read", &json!({"path": "/a"})).detected);
        assert!(!d.observe("read", &json!({"path": "/b"})).detected);
        assert!(!d.observe("read", &json!({"path": "/c"})).detected);
    }

    #[test]
    fn canonical_input_sorts_keys() {
        let mut d = DoomLoopDetector::new();
        let v1 = json!({"a": 1, "b": 2});
        let v2 = json!({"b": 2, "a": 1});
        assert!(!d.observe("t", &v1).detected);
        assert!(!d.observe("t", &v2).detected);
        let v = d.observe("t", &v1);
        assert!(v.detected, "different key order should canonicalize equal");
    }

    #[test]
    fn reset_clears_state() {
        let mut d = DoomLoopDetector::new();
        let input = json!({"x": 1});
        d.observe("t", &input);
        d.observe("t", &input);
        assert!(d.observe("t", &input).detected);

        d.reset();
        // A single call after reset isn't enough to trip the detector.
        assert!(!d.observe("t", &input).detected);
    }

    #[test]
    fn nested_objects_canonicalize_recursively() {
        let mut d = DoomLoopDetector::new();
        let v1 = json!({"a": {"x": 1, "y": 2}});
        let v2 = json!({"a": {"y": 2, "x": 1}});
        let v3 = json!({"a": {"x": 1, "y": 2}});
        assert!(!d.observe("t", &v1).detected);
        assert!(!d.observe("t", &v2).detected);
        assert!(d.observe("t", &v3).detected);
    }
}
