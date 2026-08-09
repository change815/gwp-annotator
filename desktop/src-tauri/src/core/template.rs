use std::collections::{BTreeMap, HashSet};

use data_encoding::HEXLOWER;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::core::error::{CoreError, CoreResult};
use crate::core::ids::stable_template_component;

pub const LABEL_TEMPLATE_FORMAT: &str = "gwa-label-template";

// These are explicit UTF-8-as-GBK artifacts, not a broad rejection of rare Han characters.
const GBK_MOJIBAKE_MARKERS: &[&str] = &["锟斤拷", "閫夐", "瀹炵"];

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
    pub fn parse_toml(input: &str) -> CoreResult<Self> {
        let template: Self = toml::from_str(input).map_err(|_| CoreError::InvalidTemplate)?;
        template.validate()?;
        Ok(template)
    }

    pub fn parse_toml_for_import(input: &str) -> CoreResult<Self> {
        validate_template_text_encoding(input)?;
        let template = Self::parse_toml(input)?;
        template.validate_import_text()?;
        Ok(template)
    }

    pub fn parse_json(input: &str) -> CoreResult<Self> {
        let template: Self = serde_json::from_str(input)?;
        template.validate()?;
        Ok(template)
    }

    pub fn parse_json_for_import(input: &str) -> CoreResult<Self> {
        validate_template_text_encoding(input)?;
        let template = Self::parse_json(input)?;
        template.validate_import_text()?;
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

    pub(crate) fn validate_import_text(&self) -> CoreResult<()> {
        if self.fields.iter().any(|field| {
            !has_safe_template_text(&field.label)
                || field
                    .options
                    .iter()
                    .any(|option| !has_safe_template_text(&option.label))
        }) {
            return Err(CoreError::InvalidTemplate);
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
    validate_template_text_encoding(input)?;
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
    template.validate_import_text()?;
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

fn validate_template_text_encoding(input: &str) -> CoreResult<()> {
    if !has_safe_template_text(input) {
        return Err(CoreError::InvalidTemplate);
    }
    Ok(())
}

fn has_safe_template_text(value: &str) -> bool {
    !value.contains('\u{FFFD}')
        && !value.chars().any(is_private_use_code_point)
        && !has_obvious_mojibake(value)
}

fn is_private_use_code_point(character: char) -> bool {
    matches!(
        character as u32,
        0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD
    )
}

fn has_obvious_mojibake(value: &str) -> bool {
    if GBK_MOJIBAKE_MARKERS
        .iter()
        .any(|marker| value.contains(marker))
    {
        return true;
    }

    let bytes = value
        .chars()
        .map(windows_1252_byte)
        .collect::<Vec<Option<u8>>>();

    bytes.iter().enumerate().any(|(index, lead)| {
        let Some(lead) = lead else {
            return false;
        };
        let second = bytes.get(index + 1).copied().flatten();
        let third = bytes.get(index + 2).copied().flatten();
        let fourth = bytes.get(index + 3).copied().flatten();

        (matches!(lead, 0xC2..=0xDF) && byte_in_range(second, 0x80, 0xBF))
            || (*lead == 0xE0
                && byte_in_range(second, 0xA0, 0xBF)
                && byte_in_range(third, 0x80, 0xBF))
            || ((matches!(lead, 0xE1..=0xEC | 0xEE..=0xEF))
                && byte_in_range(second, 0x80, 0xBF)
                && byte_in_range(third, 0x80, 0xBF))
            || (*lead == 0xED
                && byte_in_range(second, 0x80, 0x9F)
                && byte_in_range(third, 0x80, 0xBF))
            || (*lead == 0xF0
                && byte_in_range(second, 0x90, 0xBF)
                && byte_in_range(third, 0x80, 0xBF)
                && byte_in_range(fourth, 0x80, 0xBF))
            || (matches!(lead, 0xF1..=0xF3)
                && byte_in_range(second, 0x80, 0xBF)
                && byte_in_range(third, 0x80, 0xBF)
                && byte_in_range(fourth, 0x80, 0xBF))
            || (*lead == 0xF4
                && byte_in_range(second, 0x80, 0x8F)
                && byte_in_range(third, 0x80, 0xBF)
                && byte_in_range(fourth, 0x80, 0xBF))
    })
}

fn windows_1252_byte(character: char) -> Option<u8> {
    if (character as u32) <= 0xFF {
        return Some(character as u8);
    }
    match character {
        '\u{20AC}' => Some(0x80),
        '\u{201A}' => Some(0x82),
        '\u{0192}' => Some(0x83),
        '\u{201E}' => Some(0x84),
        '\u{2026}' => Some(0x85),
        '\u{2020}' => Some(0x86),
        '\u{2021}' => Some(0x87),
        '\u{02C6}' => Some(0x88),
        '\u{2030}' => Some(0x89),
        '\u{0160}' => Some(0x8A),
        '\u{2039}' => Some(0x8B),
        '\u{0152}' => Some(0x8C),
        '\u{017D}' => Some(0x8E),
        '\u{2018}' => Some(0x91),
        '\u{2019}' => Some(0x92),
        '\u{201C}' => Some(0x93),
        '\u{201D}' => Some(0x94),
        '\u{2022}' => Some(0x95),
        '\u{2013}' => Some(0x96),
        '\u{2014}' => Some(0x97),
        '\u{02DC}' => Some(0x98),
        '\u{2122}' => Some(0x99),
        '\u{0161}' => Some(0x9A),
        '\u{203A}' => Some(0x9B),
        '\u{0153}' => Some(0x9C),
        '\u{017E}' => Some(0x9E),
        '\u{0178}' => Some(0x9F),
        _ => None,
    }
}

fn byte_in_range(byte: Option<u8>, lower: u8, upper: u8) -> bool {
    byte.is_some_and(|byte| (lower..=upper).contains(&byte))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{parse_label_txt, FieldControl, LabelTemplate, LabelValue};

    const TOML_TEMPLATE: &str = r#"
format = "gwa-label-template"
version = 1
template_id = "template-beta"

[[fields]]
id = "field_a"
label = "字段甲"
control = "dropdown"
required = true

[[fields.options]]
id = "a1"
label = "选项甲"

[[fields]]
id = "field_b"
label = "字段乙"
control = "single_select"
required = false

[[fields.options]]
id = "b1"
label = "选项乙"

[[fields]]
id = "field_c"
label = "字段丙"
control = "multi_select"
required = true

[[fields.options]]
id = "c1"
label = "选项丙"

[[fields.options]]
id = "c2"
label = "选项丁"
"#;

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
        let template = LabelTemplate::parse_json_for_import(
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
        labels.insert("field_b".to_owned(), LabelValue::Single("b1".to_owned()));
        labels.insert(
            "field_c".to_owned(),
            LabelValue::Multiple(vec!["c1".to_owned(), "c2".to_owned()]),
        );
        assert!(template.validate_label_values(&labels).is_ok());
    }

    #[test]
    fn toml_template_controls_and_required_values_are_validated() {
        let template =
            LabelTemplate::parse_toml_for_import(TOML_TEMPLATE).expect("valid TOML template");

        assert_eq!(template.fields[0].label, "字段甲");
        assert_eq!(template.fields[0].options[0].label, "选项甲");
        assert_eq!(template.fields[0].control, FieldControl::Dropdown);
        assert_eq!(template.fields[1].control, FieldControl::SingleSelect);
        assert_eq!(template.fields[2].control, FieldControl::MultiSelect);
        assert!(template.validate_label_values(&BTreeMap::new()).is_err());

        let mut labels = BTreeMap::new();
        labels.insert("field_a".to_owned(), LabelValue::Single("a1".to_owned()));
        labels.insert("field_b".to_owned(), LabelValue::Single("b1".to_owned()));
        labels.insert(
            "field_c".to_owned(),
            LabelValue::Multiple(vec!["c1".to_owned(), "c2".to_owned()]),
        );
        assert!(template.validate_label_values(&labels).is_ok());

        labels.insert(
            "field_b".to_owned(),
            LabelValue::Multiple(vec!["b1".to_owned()]),
        );
        assert!(template.validate_label_values(&labels).is_err());
    }

    #[test]
    fn committed_study_template_matches_the_doctor_approved_radio_labels() {
        let template = LabelTemplate::parse_toml_for_import(include_str!(
            "../../../templates/label-template-v1.toml"
        ))
        .expect("committed study template");

        assert_eq!(template.template_id, "gastric-serosa-white-plaque-v1");
        assert_eq!(template.version, 1);
        assert_eq!(template.fields.len(), 3);
        assert!(template
            .fields
            .iter()
            .all(|field| field.required && field.control == FieldControl::SingleSelect));
        assert_eq!(
            template
                .fields
                .iter()
                .map(|field| field.label.as_str())
                .collect::<Vec<_>>(),
            vec!["浆膜分型", "浆膜白斑分型", "浆膜白斑置信度"]
        );
        assert_eq!(
            template.fields[0]
                .options
                .iter()
                .map(|option| option.label.as_str())
                .collect::<Vec<_>>(),
            vec![
                "正常型",
                "反应型",
                "突出结节型",
                "扁平结节型",
                "腱状型",
                "多彩弥漫型",
            ]
        );
        assert_eq!(
            template.fields[1]
                .options
                .iter()
                .map(|option| option.label.as_str())
                .collect::<Vec<_>>(),
            vec!["突出型", "扁平型", "凹陷型"]
        );
        assert_eq!(
            template.fields[2]
                .options
                .iter()
                .map(|option| option.label.as_str())
                .collect::<Vec<_>>(),
            vec!["50%", "80%", "100%"]
        );
    }

    #[test]
    fn toml_template_rejects_invalid_syntax_and_invalid_template_data() {
        assert!(LabelTemplate::parse_toml("format = [").is_err());
        assert!(LabelTemplate::parse_toml(
            r#"
format = "gwa-label-template"
version = 1
template_id = "template-invalid"

[[fields]]
id = "field_a"
label = "Field A"
control = "unsupported"
required = true
"#,
        )
        .is_err());
        assert!(LabelTemplate::parse_toml(
            r#"
format = "gwa-label-template"
version = 1
template_id = "template-invalid"

[[fields]]
id = "field_a"
label = "Field A"
control = "dropdown"
required = true
"#,
        )
        .is_err());
    }

    #[test]
    fn template_parsers_reject_encoding_damage() {
        let mojibake = "\u{00E6}\u{00B5}\u{2039}\u{00E8}\u{00AF}\u{2022}";
        for damaged_label in ["字段\u{FFFD}", "字段\u{E000}", mojibake, "閫夐」"] {
            let damaged_toml = TOML_TEMPLATE.replace("字段甲", damaged_label);
            assert!(LabelTemplate::parse_toml_for_import(&damaged_toml).is_err());
        }

        assert!(LabelTemplate::parse_json_for_import(
            r#"{
                "format":"gwa-label-template",
                "version":1,
                "template_id":"template-beta",
                "fields":[{
                    "id":"field_a",
                    "label":"字段\uFFFD",
                    "control":"dropdown",
                    "required":true,
                    "options":[{"id":"a1","label":"选项甲"}]
                }]
            }"#,
        )
        .is_err());
        assert!(parse_label_txt("字段\u{FFFD}=选项甲", "template-alpha", 1).is_err());
    }

    #[test]
    fn historical_template_parsers_keep_damaged_labels_for_verified_recovery() {
        let damaged_toml = TOML_TEMPLATE.replace("字段甲", "閫夐」");
        let template = LabelTemplate::parse_toml(&damaged_toml)
            .expect("historical fixed template remains structurally valid");
        let snapshot = template.snapshot().expect("historical snapshot");
        let restored = LabelTemplate::parse_json(&snapshot.canonical_json)
            .expect("historical SQLite snapshot remains structurally valid");

        assert_eq!(restored.fields[0].label, "閫夐」");
        assert!(LabelTemplate::parse_toml_for_import(&damaged_toml).is_err());
    }
}
