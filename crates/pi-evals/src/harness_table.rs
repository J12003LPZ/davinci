//! TS `vendor/pi/packages/evals/src/vitest-evals/harness-table.ts`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const EVAL_HARNESS_ITERATION_ARTIFACT: &str = "vitestEvalsHarnessIteration";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvalHarnessIterationArtifact {
    pub schema_version: u32,
    pub eval_set: String,
    pub group_key: String,
    pub harness: String,
    pub baseline: String,
    pub candidates: Vec<String>,
    pub repetition: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalHarnessTableRow {
    pub name: String,
    pub repetition: i64,
    pub eval_set: String,
    pub baseline: String,
    pub candidates: Vec<String>,
}

pub fn parse_eval_harness_iteration_artifact(
    value: Option<&Value>,
) -> Option<EvalHarnessIterationArtifact> {
    let value = value?;
    if !value.is_object() {
        return None;
    }
    let schema_version = value.get("schemaVersion")?.as_u64()?;
    if schema_version != 1 {
        return None;
    }
    let eval_set = value.get("evalSet")?.as_str()?.to_string();
    let group_key = value.get("groupKey")?.as_str()?.to_string();
    let harness = value.get("harness")?.as_str()?.to_string();
    let baseline = value.get("baseline")?.as_str()?.to_string();
    let candidates = value
        .get("candidates")?
        .as_array()?
        .iter()
        .map(|item| item.as_str().map(str::to_string))
        .collect::<Option<Vec<String>>>()?;
    let repetition = value.get("repetition")?.as_i64()?;
    Some(EvalHarnessIterationArtifact {
        schema_version: 1,
        eval_set,
        group_key,
        harness,
        baseline,
        candidates,
        repetition,
    })
}

fn canonicalize_json(value: &Value) -> Result<Value, String> {
    match value {
        Value::Null | Value::Bool(_) | Value::String(_) => Ok(value.clone()),
        Value::Number(number) => {
            if number.as_i64().is_some() || number.as_u64().is_some() {
                return Ok(value.clone());
            }
            if number.as_f64().is_some_and(f64::is_finite) {
                return Ok(value.clone());
            }
            Err("Eval input must contain only finite numbers.".into())
        }
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(canonicalize_json(item)?);
            }
            Ok(Value::Array(out))
        }
        Value::Object(map) => {
            let mut entries = map
                .iter()
                .map(|(key, item)| Ok((key.clone(), canonicalize_json(item)?)))
                .collect::<Result<Vec<_>, String>>()?;
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            Ok(Value::Object(entries.into_iter().collect()))
        }
    }
}

fn derive_input_key(input: &Value) -> Result<String, String> {
    if let Some(id) = input.get("id").and_then(Value::as_str) {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    let canonical = serde_json::to_string(&canonicalize_json(input)?)
        .map_err(|_| "Eval input must be JSON-serializable.".to_string())?;
    Ok(Sha256::digest(canonical.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub fn derive_eval_group_key(input: &Value, repetition: i64) -> Result<String, String> {
    let key = derive_input_key(input)?;
    Ok(serde_json::json!([key, repetition]).to_string())
}

pub fn eval_input_not_plain_object() -> &'static str {
    "Eval input must contain only plain objects and arrays."
}

pub fn eval_input_sparse_array() -> &'static str {
    "Eval input arrays must not be sparse."
}

pub fn eval_input_circular() -> &'static str {
    "Eval input must not contain circular references."
}

pub fn eval_harness_table(
    eval_set: &str,
    baseline: &str,
    candidates: &[&str],
    repetitions: i64,
) -> Result<Vec<EvalHarnessTableRow>, String> {
    if eval_set.trim().is_empty() {
        return Err("evalSet must not be empty.".into());
    }
    if candidates.is_empty() {
        return Err("At least one candidate harness is required.".into());
    }
    let mut names = vec![baseline];
    names.extend(candidates.iter().copied());
    let unique = names
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    if unique.len() != names.len() {
        return Err("Harness names must be unique within an eval set.".into());
    }
    if repetitions < 1 {
        return Err("repetitions must be a positive integer.".into());
    }
    let candidate_names: Vec<String> = candidates.iter().map(|name| (*name).to_string()).collect();
    let mut rows = Vec::new();
    for repetition in 1..=repetitions {
        for name in &names {
            rows.push(EvalHarnessTableRow {
                name: (*name).to_string(),
                repetition,
                eval_set: eval_set.to_string(),
                baseline: baseline.to_string(),
                candidates: candidate_names.clone(),
            });
        }
    }
    Ok(rows)
}

pub fn iteration_artifact_for_row(
    row: &EvalHarnessTableRow,
    input: &Value,
) -> Result<EvalHarnessIterationArtifact, String> {
    Ok(EvalHarnessIterationArtifact {
        schema_version: 1,
        eval_set: row.eval_set.clone(),
        group_key: derive_eval_group_key(input, row.repetition)?,
        harness: row.name.clone(),
        baseline: row.baseline.clone(),
        candidates: row.candidates.clone(),
        repetition: row.repetition,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn derive_group_key_and_table_lock_ts() {
        assert_eq!(
            derive_eval_group_key(&json!({"id": " input-1 ", "prompt": "hello"}), 2).unwrap(),
            serde_json::json!(["input-1", 2]).to_string()
        );
        let left =
            derive_eval_group_key(&json!({"first": 1, "second": [true, "value"]}), 1).unwrap();
        let right =
            derive_eval_group_key(&json!({"second": [true, "value"], "first": 1}), 1).unwrap();
        assert_eq!(left, right);
        assert_ne!(
            derive_eval_group_key(&json!({"first": 1}), 1).unwrap(),
            derive_eval_group_key(&json!({"first": 2}), 1).unwrap()
        );
        assert_ne!(
            derive_eval_group_key(&json!({"first": 1}), 1).unwrap(),
            derive_eval_group_key(&json!({"first": 1}), 2).unwrap()
        );
        assert_ne!(
            derive_eval_group_key(&json!(["first", "second"]), 1).unwrap(),
            derive_eval_group_key(&json!(["second", "first"]), 1).unwrap()
        );
        assert_eq!(
            eval_input_not_plain_object(),
            "Eval input must contain only plain objects and arrays."
        );
        assert_eq!(
            eval_input_sparse_array(),
            "Eval input arrays must not be sparse."
        );
        assert_eq!(
            eval_input_circular(),
            "Eval input must not contain circular references."
        );

        let table = eval_harness_table(
            "local multi-harness eval",
            "withoutSkill",
            &["withSkill"],
            2,
        )
        .unwrap();
        let planned: Vec<(&str, i64)> = table
            .iter()
            .map(|row| (row.name.as_str(), row.repetition))
            .collect();
        assert_eq!(
            planned,
            vec![
                ("withoutSkill", 1),
                ("withSkill", 1),
                ("withoutSkill", 2),
                ("withSkill", 2),
            ]
        );
        let singular = eval_harness_table("singular candidate", "baseline", &["candidate"], 1)
            .unwrap()
            .into_iter()
            .map(|row| row.name)
            .collect::<Vec<_>>();
        assert_eq!(singular, vec!["baseline", "candidate"]);
        for row in &table {
            let artifact = iteration_artifact_for_row(row, &json!({"id": "first"})).unwrap();
            assert_eq!(
                parse_eval_harness_iteration_artifact(Some(
                    &serde_json::to_value(&artifact).unwrap()
                )),
                Some(EvalHarnessIterationArtifact {
                    schema_version: 1,
                    eval_set: "local multi-harness eval".into(),
                    group_key: derive_eval_group_key(&json!({"id": "first"}), row.repetition)
                        .unwrap(),
                    harness: row.name.clone(),
                    baseline: "withoutSkill".into(),
                    candidates: vec!["withSkill".into()],
                    repetition: row.repetition,
                })
            );
        }
        assert_eq!(
            eval_harness_table(" ", "a", &["b"], 1).unwrap_err(),
            "evalSet must not be empty."
        );
        assert_eq!(
            eval_harness_table("set", "a", &[], 1).unwrap_err(),
            "At least one candidate harness is required."
        );
        assert_eq!(
            eval_harness_table("set", "same", &["same"], 1).unwrap_err(),
            "Harness names must be unique within an eval set."
        );
        assert_eq!(
            eval_harness_table("set", "a", &["b"], 0).unwrap_err(),
            "repetitions must be a positive integer."
        );
    }
}
