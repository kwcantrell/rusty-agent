use crate::ToolSchema;

/// Marker `ToolRegistry::schemas()` prepends before folded exclusion prose; also
/// the string the enforcement tests grep for.
pub const WHEN_NOT_TO_CALL_MARKER: &str = "When NOT to call:";

/// Tools genuinely confusable with a sibling that MUST carry `when_not_to_call`
/// prose. A maintained ratchet — add a new confusable tool here by hand.
/// Clusters: recall/context_recall (semantic memory vs offload rehydration),
/// read_file/read_skill_file (workspace vs skill dir), write_file/edit_file
/// (create-or-overwrite vs unique-substring replace),
/// execute_command/read_file+list_directory+git_* (a shell subsumes the
/// dedicated Read-tier tools but at Write-tier friction).
/// NOTE: `recall` is runtime-injected, so it is enforced in agent-memory's own
/// test rather than the agent-runtime-config enforcement test.
pub const CONFUSABLE_TOOLS: &[&str] = &[
    "recall",
    "context_recall",
    "read_file",
    "read_skill_file",
    "write_file",
    "edit_file",
    "execute_command",
];

/// Names of `schema`'s required params whose `properties[name].description` is
/// missing or empty, including required params of array-`items` object schemas
/// (reported as `parent[].child`). Empty vec = compliant.
/// Scope is deliberately array-items only (audit 2.4): plain object properties
/// with their own `required` don't occur in our schemas, and recursing into
/// them would flood the warn-only MCP connect-time lint.
pub fn required_params_missing_description(schema: &ToolSchema) -> Vec<String> {
    let mut out = Vec::new();
    collect_missing(&schema.parameters, "", &mut out);
    out
}

fn collect_missing(obj: &serde_json::Value, prefix: &str, out: &mut Vec<String>) {
    let props = obj.get("properties").and_then(|v| v.as_object());
    if let Some(required) = obj.get("required").and_then(|r| r.as_array()) {
        for name in required.iter().filter_map(|v| v.as_str()) {
            let desc = props
                .and_then(|o| o.get(name))
                .and_then(|prop| prop.get("description"))
                .and_then(|d| d.as_str());
            if desc.map(|s| s.trim().is_empty()).unwrap_or(true) {
                out.push(format!("{prefix}{name}"));
            }
        }
    }
    for (name, prop) in props.into_iter().flatten() {
        if let Some(items) = prop.get("items") {
            if items.get("properties").is_some() {
                collect_missing(items, &format!("{prefix}{name}[]."), out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema(parameters: serde_json::Value) -> ToolSchema {
        ToolSchema {
            name: "t".into(),
            description: "d".into(),
            parameters,
        }
    }

    #[test]
    fn flags_required_param_without_description() {
        let s = schema(json!({"type":"object",
            "properties":{"path":{"type":"string"}}, "required":["path"]}));
        assert_eq!(
            required_params_missing_description(&s),
            vec!["path".to_string()]
        );
    }

    #[test]
    fn empty_description_counts_as_missing() {
        let s = schema(json!({"type":"object",
            "properties":{"path":{"type":"string","description":"  "}}, "required":["path"]}));
        assert_eq!(
            required_params_missing_description(&s),
            vec!["path".to_string()]
        );
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

    #[test]
    fn nested_array_item_required_params_are_flagged() {
        // Audit 2.4: required params inside array `items` object schemas are
        // part of the tool contract too.
        let s = schema(json!({"type":"object",
            "properties":{
                "files":{"type":"array","description":"bundled files","items":{
                    "type":"object",
                    "properties":{
                        "path":{"type":"string"},
                        "content":{"type":"string","description":"file body"}},
                    "required":["path","content"]}}},
            "required":[]}));
        assert_eq!(
            required_params_missing_description(&s),
            vec!["files[].path".to_string()]
        );
    }

    #[test]
    fn described_nested_array_item_params_are_compliant() {
        let s = schema(json!({"type":"object",
            "properties":{
                "files":{"type":"array","description":"bundled files","items":{
                    "type":"object",
                    "properties":{
                        "path":{"type":"string","description":"where"},
                        "content":{"type":"string","description":"what"}},
                    "required":["path","content"]}}},
            "required":[]}));
        assert!(required_params_missing_description(&s).is_empty());
    }

    #[test]
    fn string_items_arrays_are_ignored() {
        // Arrays of scalars (tags, columns) have no nested contract to check.
        let s = schema(json!({"type":"object",
            "properties":{"tags":{"type":"array","description":"labels",
                "items":{"type":"string"}}},
            "required":[]}));
        assert!(required_params_missing_description(&s).is_empty());
    }
}
