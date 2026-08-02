use std::collections::{BTreeMap, HashSet};

use data_encoding::HEXLOWER;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::core::error::{CoreError, CoreResult};
use crate::core::ids::stable_template_component;

pub const LABEL_TEMPLATE_FORMAT: &str = "gwa-label-template";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct LabelTemplate {
    pub format: String,
    pub version: u32,
    pub template_id: String,
    pub fields: Vec<TemplateField>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TemplateField {
    pub id: String,
    pub label: String,
    pub control: FieldControl,
    pub required: bool,
    pub options: Vec<TemplateOption>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TemplateOption {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FieldControl {
    Dropdown,
    SingleSelect,
    MultiSelect,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum LabelValue {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TemplateSnapshot {
    pub template_id: String,
    pub version: u32,
    pub sha256: String,
    pub canonical_json: String,
}

impl LabelTemplate {
    pub fn parse_json(input: &str) -> CoreResult<Self> {
        let template: Self = serde_json::from_str(input)?;
        template.validate()?;
        Ok(template)
    }

    pub fn validate(&self) -> CoreResult<()> {
        if self.format != LABEL_TEMPLATE_FORMAT
            || self.version == 0
            || !is_machine_id(&self.template_id)
            || self.fields.is_empty()
        {
            return Err(CoreError::InvalidTemplate);
        }

        let mut field_ids = HashSet::new();
        for field in &self.fields {
            if !is_machine_id(&field.id)
                || field.label.trim().is_empty()
                || !field_ids.insert(field.id.as_str())
                || (field.required && field.options.is_empty())
            {
                return Err(CoreError::InvalidTemplate);
            }

            let mut option_ids = HashSet::new();
            for option in &field.options {
                if !is_machine_id(&option.id)
                    || option.label.trim().is_empty()
                    || !option_ids.insert(option.id.as_str())
                {
                    return Err(CoreError::InvalidTemplate);
                }
            }
        }

        Ok(())
    }

    pub fn canonical_json(&self) -> CoreResult<String> {
        self.validate()?;
        serde_json::to_string(self).map_err(Into::into)
    }

    pub fn snapshot(&self) -> CoreResult<TemplateSnapshot> {
        let canonical_json = self.canonical_json()?;
        let sha256 = HEXLOWER.encode(Sha256::digest(canonical_json.as_bytes()).as_slice());
        Ok(TemplateSnapshot {
            template_id: self.template_id.clone(),
            version: self.version,
            sha256,
            canonical_json,
        })
    }

    pub fn validate_label_values(&self, labels: &BTreeMap<String, LabelValue>) -> CoreResult<()> {
        self.validate()?;

        for field_id in labels.keys() {
            if !self.fields.iter().any(|field| &field.id == field_id) {
                return Err(CoreError::InvalidLabels);
            }
        }

        for field in &self.fields {
            match labels.get(&field.id) {
                None if field.required => return Err(CoreError::InvalidLabels),
                None => continue,
                Some(value) => validate_field_value(field, value)?,
            }
        }

        Ok(())
    }
}

pub fn parse_label_txt(input: &str, template_id: &str, version: u32) -> CoreResult<LabelTemplate> {
    let mut fields = Vec::new();
    let mut seen_display_names = HashSet::new();

    for raw_line in input.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let (required, body) = match trimmed.strip_prefix('*') {
            Some(value) => (true, value.trim()),
            None => (false, trimmed),
        };
        let (display_name, raw_options) = body.split_once('=').ok_or(CoreError::InvalidTemplate)?;
        let display_name = display_name.trim();
        if display_name.is_empty() || !seen_display_names.insert(display_name.to_owned()) {
            return Err(CoreError::InvalidTemplate);
        }

        let field_id = stable_template_component("field", display_name);
        let mut options = Vec::new();
        let mut seen_option_names = HashSet::new();
        for raw_option in raw_options.split('|') {
            let option_label = raw_option.trim();
            if option_label.is_empty() || !seen_option_names.insert(option_label.to_owned()) {
                return Err(CoreError::InvalidTemplate);
            }
            options.push(TemplateOption {
                id: stable_template_component("option", &format!("{field_id}\0{option_label}")),
                label: option_label.to_owned(),
            });
        }

        fields.push(TemplateField {
            id: field_id,
            label: display_name.to_owned(),
            control: FieldControl::Dropdown,
            required,
            options,
        });
    }

    let template = LabelTemplate {
        format: LABEL_TEMPLATE_FORMAT.to_owned(),
        version,
        template_id: template_id.to_owned(),
        fields,
    };
    template.validate()?;
    Ok(template)
}

fn validate_field_value(field: &TemplateField, value: &LabelValue) -> CoreResult<()> {
    let values = match (&field.control, value) {
        (FieldControl::Dropdown | FieldControl::SingleSelect, LabelValue::Single(value)) => {
            std::slice::from_ref(value)
        }
        (FieldControl::MultiSelect, LabelValue::Multiple(values)) if !values.is_empty() => values,
        _ => return Err(CoreError::InvalidLabels),
    };

    let mut seen = HashSet::new();
    if values.iter().any(|value| {
        !seen.insert(value.as_str()) || !field.options.iter().any(|option| &option.id == value)
    }) {
        return Err(CoreError::InvalidLabels);
    }
    Ok(())
}

fn is_machine_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(byte) if byte.is_ascii_alphabetic())
        && value.len() <= 64
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{parse_label_txt, FieldControl, LabelTemplate, LabelValue};

    #[test]
    fn label_txt_import_validation() {
        let template = parse_label_txt(
            "# comment\n*Field A=Option A|Option B\nField B=Option C",
            "template-alpha",
            1,
        )
        .expect("valid label text");
        assert_eq!(template.fields.len(), 2);
        assert!(template.fields[0].required);
        assert_eq!(template.fields[0].control, FieldControl::Dropdown);
        assert!(parse_label_txt("*Field A=", "template-alpha", 1).is_err());
        assert!(
            parse_label_txt("Field A=Option A\nField A=Option B", "template-alpha", 1).is_err()
        );
        assert!(parse_label_txt("Field A=Option A|Option A", "template-alpha", 1).is_err());
    }

    #[test]
    fn json_template_controls_and_required_values_are_validated() {
        let template = LabelTemplate::parse_json(
            r#"{
                "format":"gwa-label-template",
                "version":1,
                "template_id":"template-beta",
                "fields":[
                  {"id":"field_a","label":"Field A","control":"dropdown","required":true,"options":[{"id":"a1","label":"Option A"}]},
                  {"id":"field_b","label":"Field B","control":"single_select","required":false,"options":[{"id":"b1","label":"Option B"}]},
                  {"id":"field_c","label":"Field C","control":"multi_select","required":true,"options":[{"id":"c1","label":"Option C"},{"id":"c2","label":"Option D"}]}
                ]
            }"#,
        )
        .expect("valid JSON template");

        assert!(template.validate_label_values(&BTreeMap::new()).is_err());
        let mut labels = BTreeMap::new();
        labels.insert("field_a".to_owned(), LabelValue::Single("a1".to_owned()));
        labels.insert(
            "field_c".to_owned(),
            LabelValue::Multiple(vec!["c1".to_owned(), "c2".to_owned()]),
        );
        assert!(template.validate_label_values(&labels).is_ok());
    }
}
