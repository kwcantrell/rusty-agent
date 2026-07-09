//! Structured sub-agent responses (spec 3B-1b). A named sub-agent may declare a
//! FLAT-object `response_format`; the synthetic `respond` tool validates the
//! child's structured answer against it and writes it to a shared handle. No
//! nesting, no regex, no recursion — validation is a single flat pass.
use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// The single structured payload a `respond` call captures, shared between the
/// `RespondTool` (writer) and `ResponseCapture` / dispatch handoff (readers).
/// Mirrors `TodoHandle`. `None` until a valid `respond` call lands.
pub type ResponseHandle = Arc<Mutex<Option<Value>>>;

/// Max declared properties on a `response_format` object (flat dialect ceiling).
pub const MAX_RESPONSE_SCHEMA_PROPERTIES: usize = 64;

const SCALAR_TYPES: [&str; 5] = ["string", "number", "integer", "boolean", "null"];
const BANNED_KEYS: [&str; 8] = [
    "pattern", "$ref", "allOf", "anyOf", "oneOf", "not", "$defs", "format",
];

/// Config-time: is `schema` a well-formed FLAT-object response_format? (spec §2.5)
pub fn validate_schema(schema: &Value) -> Result<(), String> {
    let obj = schema.as_object().ok_or("must be a JSON object")?;
    if obj.get("type").and_then(Value::as_str) != Some("object") {
        return Err("top-level `type` must be \"object\"".into());
    }
    if obj.get("additionalProperties") != Some(&Value::Bool(false)) {
        return Err("must set `additionalProperties: false` (closed object)".into());
    }
    for k in BANNED_KEYS {
        if obj.contains_key(k) {
            return Err(format!("key `{k}` is not allowed"));
        }
    }
    let props = obj
        .get("properties")
        .and_then(Value::as_object)
        .ok_or("must have an object `properties`")?;
    if props.len() > MAX_RESPONSE_SCHEMA_PROPERTIES {
        return Err(format!(
            "too many properties ({} > {MAX_RESPONSE_SCHEMA_PROPERTIES})",
            props.len()
        ));
    }
    for (name, sub) in props {
        validate_property(name, sub)?;
    }
    if let Some(req) = obj.get("required") {
        let arr = req.as_array().ok_or("`required` must be an array")?;
        for r in arr {
            let rn = r.as_str().ok_or("`required` entries must be strings")?;
            if !props.contains_key(rn) {
                return Err(format!("`required` names unknown property `{rn}`"));
            }
        }
    }
    Ok(())
}

fn validate_property(name: &str, sub: &Value) -> Result<(), String> {
    let o = sub
        .as_object()
        .ok_or_else(|| format!("property `{name}` must be a schema object"))?;
    for k in BANNED_KEYS {
        if o.contains_key(k) {
            return Err(format!("property `{name}`: key `{k}` is not allowed"));
        }
    }
    match o.get("type").and_then(Value::as_str) {
        Some("object") => Err(format!("property `{name}`: nested object not allowed")),
        Some("array") => {
            let items = o
                .get("items")
                .and_then(Value::as_object)
                .ok_or_else(|| format!("property `{name}`: array needs object `items`"))?;
            let it = items.get("type").and_then(Value::as_str);
            if it == Some("object") || items.contains_key("properties") {
                return Err(format!("property `{name}`: array-of-object not allowed"));
            }
            if !matches!(it, Some(t) if SCALAR_TYPES.contains(&t)) {
                return Err(format!(
                    "property `{name}`: array `items.type` must be scalar"
                ));
            }
            Ok(())
        }
        Some(t) if SCALAR_TYPES.contains(&t) => Ok(()),
        Some(t) => Err(format!("property `{name}`: unsupported type `{t}`")),
        None if o.contains_key("enum") => Ok(()), // enum-only (scalar literals)
        None => Err(format!("property `{name}`: needs a `type` or `enum`")),
    }
}

/// Runtime: does `payload` conform to the already-validated flat `schema`? (spec §2.5)
pub fn validate_payload(schema: &Value, payload: &Value) -> Result<(), String> {
    let sobj = schema.as_object().ok_or("schema not an object")?;
    let props = sobj
        .get("properties")
        .and_then(Value::as_object)
        .ok_or("schema has no properties")?;
    let p = payload
        .as_object()
        .ok_or("response must be a JSON object")?;
    if let Some(req) = sobj.get("required").and_then(Value::as_array) {
        for r in req {
            if let Some(rn) = r.as_str() {
                if !p.contains_key(rn) {
                    return Err(format!("missing required key `{rn}`"));
                }
            }
        }
    }
    for k in p.keys() {
        if !props.contains_key(k) {
            return Err(format!("unexpected property `{k}`"));
        }
    }
    for (name, val) in p {
        // `.get()` not `props[name]` — panic-free even if a caller ever passes an
        // unvalidated schema; the closed-object check above already guarantees the key.
        let sub = props
            .get(name)
            .and_then(Value::as_object)
            .ok_or("schema property not an object")?;
        check_value(name, sub, val)?;
    }
    Ok(())
}

fn check_value(
    name: &str,
    sub: &serde_json::Map<String, Value>,
    val: &Value,
) -> Result<(), String> {
    if let Some(e) = sub.get("enum").and_then(Value::as_array) {
        if !e.iter().any(|lit| lit == val) {
            return Err(format!("property `{name}`: value not in enum"));
        }
        return Ok(());
    }
    let scalar_ok = |t: Option<&str>, v: &Value| match t {
        Some("string") => v.is_string(),
        Some("boolean") => v.is_boolean(),
        Some("null") => v.is_null(),
        Some("integer") => v.is_i64() || v.is_u64(),
        Some("number") => v.is_number(),
        _ => false,
    };
    match sub.get("type").and_then(Value::as_str) {
        Some("array") => {
            let arr = val
                .as_array()
                .ok_or_else(|| format!("property `{name}`: expected array"))?;
            let it = sub
                .get("items")
                .and_then(Value::as_object)
                .and_then(|i| i.get("type"))
                .and_then(Value::as_str);
            for (i, el) in arr.iter().enumerate() {
                if !scalar_ok(it, el) {
                    return Err(format!("property `{name}`[{i}]: wrong element type"));
                }
            }
            Ok(())
        }
        t if scalar_ok(t, val) => Ok(()),
        _ => Err(format!("property `{name}`: wrong scalar type")),
    }
}

/// The reserved name of the synthetic structured-response tool (spec §2.2).
pub const RESPOND_TOOL_NAME: &str = "respond";

/// The synthetic tool a named child with a `response_format` uses to return its
/// structured answer. Validates args against the flat schema and writes the
/// payload to the shared handle. A pure leaf: no dispatch power, no workspace
/// side-effects (spec §3 inv. 7).
pub struct RespondTool {
    schema: Value,
    handle: ResponseHandle,
    description: String,
}

impl RespondTool {
    pub fn new(schema: Value, handle: ResponseHandle) -> Self {
        Self {
            schema,
            handle,
            description: "Return your final answer as structured data matching this \
                tool's schema. Call this exactly once when the task is complete; its \
                arguments are returned to the parent as the result. If a call is \
                rejected as invalid, correct the arguments and call it again."
                .into(),
        }
    }
}

#[async_trait]
impl Tool for RespondTool {
    fn name(&self) -> &str {
        RESPOND_TOOL_NAME
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: RESPOND_TOOL_NAME.into(),
            description: self.description.clone(),
            parameters: self.schema.clone(),
        }
    }
    fn intent(&self, _args: &Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: RESPOND_TOOL_NAME.into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "return the structured response".into(),
        })
    }
    async fn execute(&self, args: Value, _ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        validate_payload(&self.schema, &args)
            .map_err(|e| ToolError::InvalidArgs(format!("respond: {e}")))?;
        *self.handle.lock().unwrap() = Some(args);
        Ok(ToolOutput {
            content: "response recorded".into(),
            display: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn good_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["severity", "files"],
            "properties": {
                "severity": {"type": "string", "enum": ["low", "high"]},
                "files": {"type": "array", "items": {"type": "string"}},
                "count": {"type": "integer"}
            }
        })
    }

    #[test]
    fn accepts_flat_object_schema() {
        assert!(validate_schema(&good_schema()).is_ok());
    }

    #[test]
    fn rejects_non_object_and_bad_top_level() {
        assert!(validate_schema(&json!([1, 2])).is_err());
        assert!(validate_schema(&json!({"type": "string"})).is_err());
        assert!(validate_schema(&json!({"type": "object", "properties": {}})).is_err());
        // no additionalProperties:false
    }

    #[test]
    fn rejects_nesting_and_array_of_object() {
        let nested = json!({"type":"object","additionalProperties":false,
            "properties":{"inner":{"type":"object","additionalProperties":false,"properties":{}}}});
        assert!(validate_schema(&nested).is_err());
        let aoo = json!({"type":"object","additionalProperties":false,
            "properties":{"rows":{"type":"array","items":{"type":"object"}}}});
        assert!(validate_schema(&aoo).is_err());
    }

    #[test]
    fn rejects_regex_and_combinators() {
        let pat = json!({"type":"object","additionalProperties":false,
            "properties":{"s":{"type":"string","pattern":"^a+$"}}});
        assert!(validate_schema(&pat).is_err());
        let comb = json!({"type":"object","additionalProperties":false,
            "anyOf":[], "properties":{}});
        assert!(validate_schema(&comb).is_err());
    }

    #[test]
    fn rejects_required_naming_unknown_property_and_over_cap() {
        let bad_req = json!({"type":"object","additionalProperties":false,
            "required":["ghost"], "properties":{"real":{"type":"string"}}});
        assert!(validate_schema(&bad_req).is_err());
        let mut props = serde_json::Map::new();
        for i in 0..(MAX_RESPONSE_SCHEMA_PROPERTIES + 1) {
            props.insert(format!("p{i}"), json!({"type": "string"}));
        }
        let over = json!({"type":"object","additionalProperties":false,"properties":props});
        assert!(validate_schema(&over).is_err());
    }

    #[test]
    fn payload_valid_and_invalid() {
        let s = good_schema();
        assert!(validate_payload(&s, &json!({"severity":"low","files":["a.rs"]})).is_ok());
        assert!(validate_payload(&s, &json!({"files":["a.rs"]})).is_err()); // missing required
        assert!(validate_payload(&s, &json!({"severity":"low","files":["a.rs"],"x":1})).is_err()); // unknown key
        assert!(validate_payload(&s, &json!({"severity":"nope","files":[]})).is_err()); // enum miss
        assert!(validate_payload(&s, &json!({"severity":"low","files":[3]})).is_err()); // bad array element
        assert!(validate_payload(&s, &json!({"severity":"low","files":"a.rs"})).is_err()); // not an array
        assert!(validate_payload(&s, &json!("scalar")).is_err()); // not an object
    }

    #[tokio::test]
    async fn respond_tool_writes_handle_on_valid_and_errs_on_invalid() {
        use tokio_util::sync::CancellationToken;
        let schema = good_schema();
        let handle: ResponseHandle = Arc::new(Mutex::new(None));
        let tool = RespondTool::new(schema, handle.clone());
        assert_eq!(tool.name(), RESPOND_TOOL_NAME);

        let ctx = ToolCtx {
            workspace: std::env::temp_dir(),
            timeout: std::time::Duration::from_secs(5),
            cancel: CancellationToken::new(),
            sandbox: Arc::new(agent_tools::HostExecutor),
            backend: Arc::new(agent_tools::backend::HostBackend::new(std::env::temp_dir())),
            call_id: "r1".into(),
        };

        let ok = tool
            .execute(json!({"severity": "low", "files": ["a.rs"]}), &ctx)
            .await
            .unwrap();
        assert_eq!(ok.content, "response recorded");
        assert_eq!(handle.lock().unwrap().as_ref().unwrap()["severity"], "low");

        *handle.lock().unwrap() = None;
        let err = tool
            .execute(json!({"files": ["a.rs"]}), &ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, agent_tools::ToolError::InvalidArgs(_)));
        assert!(handle.lock().unwrap().is_none());
    }
}
