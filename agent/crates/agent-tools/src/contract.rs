use crate::ToolSchema;

/// Marker `ToolRegistry::schemas()` prepends before folded exclusion prose; also
/// the string the enforcement tests grep for.
pub const WHEN_NOT_TO_CALL_MARKER: &str = "When NOT to call:";

/// Tools genuinely confusable with a sibling that MUST carry `when_not_to_call`
/// prose. A maintained ratchet — add a new confusable tool here by hand.
/// Clusters: recall/context_recall (semantic memory vs offload rehydration),
/// read_file/read_skill_file (workspace vs skill dir), write_file/edit_file
/// (create-or-overwrite vs unique-substring replace).
/// NOTE: `recall` is runtime-injected, so it is enforced in agent-memory's own
/// test rather than the agent-runtime-config enforcement test.
pub const CONFUSABLE_TOOLS: &[&str] = &[
    "recall", "context_recall", "read_file", "read_skill_file", "write_file", "edit_file",
];

/// Names of `schema`'s required params whose `properties[name].description` is
/// missing or empty. Empty vec = compliant.
pub fn required_params_missing_description(schema: &ToolSchema) -> Vec<String> {
    let params = &schema.parameters;
    let required = params.get("required").and_then(|r| r.as_array()).cloned().unwrap_or_default();
    let props = params.get("properties").and_then(|v| v.as_object());
    required
        .iter()
        .filter_map(|r| r.as_str())
        .filter(|name| {
            let desc = props
                .and_then(|o| o.get(*name))
                .and_then(|prop| prop.get("description"))
                .and_then(|d| d.as_str());
            desc.map(|s| s.trim().is_empty()).unwrap_or(true)
        })
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema(parameters: serde_json::Value) -> ToolSchema {
        ToolSchema { name: "t".into(), description: "d".into(), parameters }
    }

    #[test]
    fn flags_required_param_without_description() {
        let s = schema(json!({"type":"object",
            "properties":{"path":{"type":"string"}}, "required":["path"]}));
        assert_eq!(required_params_missing_description(&s), vec!["path".to_string()]);
    }

    #[test]
    fn empty_description_counts_as_missing() {
        let s = schema(json!({"type":"object",
            "properties":{"path":{"type":"string","description":"  "}}, "required":["path"]}));
        assert_eq!(required_params_missing_description(&s), vec!["path".to_string()]);
    }

    #[test]
    fn described_required_param_is_compliant() {
        let s = schema(json!({"type":"object",
            "properties":{"path":{"type":"string","description":"the path"}}, "required":["path"]}));
        assert!(required_params_missing_description(&s).is_empty());
    }

    #[test]
    fn optional_undescribed_param_is_ignored() {
        let s = schema(json!({"type":"object",
            "properties":{"path":{"type":"string","description":"p"},"k":{"type":"integer"}},
            "required":["path"]}));
        assert!(required_params_missing_description(&s).is_empty());
    }
}
