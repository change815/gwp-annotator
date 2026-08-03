use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use filetime::FileTime;
use gwp_annotator_core::core::error::CoreError;
use gwp_annotator_core::core::scan::{MediaKind, PublicAsset, PublicCase, PublicScan};
use gwp_annotator_core::core::service::{
    CaptureFrameRequest, ReplaceRegionRequest, RestoreStatus, SaveRegionRequest,
    UpdateRegionLabelsRequest,
};
use gwp_annotator_core::core::store::{PointNorm, RecoveryState, SaveFault};
use gwp_annotator_core::core::template::{
    FieldControl, LabelTemplate, LabelValue, TemplateField, TemplateOption, LABEL_TEMPLATE_FORMAT,
};
use gwp_annotator_core::{FixedSecretProvider, ProjectService};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

struct Fixture {
    _temporary: tempfile::TempDir,
    source_root: PathBuf,
    work_root: PathBuf,
}

fn fixture(images_per_case: usize) -> Fixture {
    let temporary = tempfile::tempdir().expect("temporary fixture");
    let source_root = temporary.path().join("ROOT-CASE-FIXTURE");
    let work_root = temporary.path().join("WORK-CASE-FIXTURE");
    let alpha_case = source_root.join("CAMPUS-ALPHA").join("20240101_CASE-ALPHA");
    let beta_case = source_root.join("CAMPUS-BETA").join("20240102_CASE-BETA");
    fs::create_dir_all(&alpha_case).expect("alpha case");
    fs::create_dir_all(&beta_case).expect("beta case");
    for index in 1..=images_per_case {
        fs::write(
            alpha_case.join(format!("ASSET-{index:02}.JPG")),
            vec![index as u8; index + 1],
        )
        .expect("fixture image");
    }
    fs::write(beta_case.join("ASSET-01.MP4"), b"fixture-video").expect("fixture video");
    Fixture {
        _temporary: temporary,
        source_root,
        work_root,
    }
}

fn service(fixture: &Fixture) -> ProjectService {
    service_with_work_root(&fixture.work_root)
}

fn service_with_work_root(work_root: &Path) -> ProjectService {
    ProjectService::new(
        work_root,
        "PROJECT-LOCAL",
        Arc::new(FixedSecretProvider::from_bytes([3; 32])),
    )
    .expect("service")
}

fn first_image(scan: &PublicScan) -> (PublicCase, PublicAsset) {
    scan.campuses
        .iter()
        .flat_map(|campus| campus.cases.iter())
        .find_map(|case| {
            case.assets
                .iter()
                .find(|asset| asset.media_kind == MediaKind::Image)
                .cloned()
                .map(|asset| (case.clone(), asset))
        })
        .expect("image asset")
}

fn all_images(scan: &PublicScan) -> Vec<(PublicCase, PublicAsset)> {
    scan.campuses
        .iter()
        .flat_map(|campus| campus.cases.iter())
        .flat_map(|case| {
            case.assets
                .iter()
                .filter(|asset| asset.media_kind == MediaKind::Image)
                .cloned()
                .map(|asset| (case.clone(), asset))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn first_video(scan: &PublicScan) -> (PublicCase, PublicAsset) {
    scan.campuses
        .iter()
        .flat_map(|campus| campus.cases.iter())
        .find_map(|case| {
            case.assets
                .iter()
                .find(|asset| asset.media_kind == MediaKind::Video)
                .cloned()
                .map(|asset| (case.clone(), asset))
        })
        .expect("video asset")
}

const V02_TOML_TEMPLATE: &str = r#"
format = "gwa-label-template"
version = 1
template_id = "template-v02"

[[fields]]
id = "field_dropdown"
label = "Dropdown"
control = "dropdown"
required = true

[[fields.options]]
id = "dropdown_a"
label = "Dropdown A"

[[fields.options]]
id = "dropdown_b"
label = "Dropdown B"

[[fields]]
id = "field_single"
label = "Single"
control = "single_select"
required = false

[[fields.options]]
id = "single_a"
label = "Single A"

[[fields]]
id = "field_multi"
label = "Multi"
control = "multi_select"
required = true

[[fields.options]]
id = "multi_a"
label = "Multi A"

[[fields.options]]
id = "multi_b"
label = "Multi B"
"#;

fn v02_labels(dropdown: &str) -> BTreeMap<String, LabelValue> {
    BTreeMap::from([
        (
            "field_dropdown".to_owned(),
            LabelValue::Single(dropdown.to_owned()),
        ),
        (
            "field_single".to_owned(),
            LabelValue::Single("single_a".to_owned()),
        ),
        (
            "field_multi".to_owned(),
            LabelValue::Multiple(vec!["multi_a".to_owned(), "multi_b".to_owned()]),
        ),
    ])
}

fn synthetic_png(width: u32, height: u32, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::from([137, 80, 78, 71, 13, 10, 26, 10]);
    bytes.extend_from_slice(&13_u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(payload);
    bytes
}

fn v02_region_request(
    region_id: &str,
    asset_id: String,
    labels: BTreeMap<String, LabelValue>,
) -> SaveRegionRequest {
    SaveRegionRequest {
        region_id: region_id.to_owned(),
        asset_id,
        polygon_norm: vec![
            PointNorm { x: 0.1, y: 0.1 },
            PointNorm { x: 0.8, y: 0.1 },
            PointNorm { x: 0.4, y: 0.8 },
        ],
        template_id: "template-v02".to_owned(),
        template_version: 1,
        labels,
    }
}

fn generic_template() -> LabelTemplate {
    LabelTemplate {
        format: LABEL_TEMPLATE_FORMAT.to_owned(),
        version: 1,
        template_id: "template-alpha".to_owned(),
        fields: vec![TemplateField {
            id: "field_alpha".to_owned(),
            label: "Field Alpha".to_owned(),
            control: FieldControl::Dropdown,
            required: true,
            options: vec![TemplateOption {
                id: "option_alpha".to_owned(),
                label: "Option Alpha".to_owned(),
            }],
        }],
    }
}

fn region_request(asset_id: String) -> SaveRegionRequest {
    SaveRegionRequest {
        region_id: "REGION-ABCDEFGH".to_owned(),
        asset_id,
        polygon_norm: vec![
            PointNorm { x: 0.1, y: 0.1 },
            PointNorm { x: 0.8, y: 0.1 },
            PointNorm { x: 0.4, y: 0.8 },
        ],
        template_id: "template-alpha".to_owned(),
        template_version: 1,
        labels: BTreeMap::from([(
            "field_alpha".to_owned(),
            LabelValue::Single("option_alpha".to_owned()),
        )]),
    }
}

fn load_generic_template(service: &ProjectService) {
    let template_json = serde_json::to_string(&generic_template()).expect("template JSON");
    service
        .import_json_template(&template_json)
        .expect("load template");
}

#[test]
fn representative_selection_has_no_limit_and_keeps_annotation_gate() {
    let fixture = fixture(4);
    let service = service(&fixture);
    let scan = service
        .scan_selected_root(&fixture.source_root)
        .expect("scan fixture");
    let images = all_images(&scan);
    let (case, first_asset) = images.first().expect("first image").clone();
    load_generic_template(&service);

    let before_selection =
        service.save_confirmed_region(region_request(first_asset.asset_id.clone()));
    assert!(matches!(
        before_selection,
        Err(CoreError::AssetNotRepresentative)
    ));

    for (index, (_, asset)) in images.iter().enumerate() {
        let selection = service
            .select_representative(&case.case_id, &asset.asset_id)
            .expect("representative selection");
        assert_eq!(selection.representative_count, index + 1);
    }
    assert_eq!(
        service
            .recorded_representative_count(&case.case_id)
            .expect("recorded count"),
        4
    );
    assert_eq!(
        service.representative_count(&case.case_id).expect("count"),
        4
    );
}

fn create_legacy_v1_database(
    fixture: &Fixture,
    case: &PublicCase,
    campus_code: &str,
    assets: &[PublicAsset],
) {
    fs::create_dir_all(&fixture.work_root).expect("legacy work root");
    let connection = Connection::open(fixture.work_root.join("annotations.sqlite3"))
        .expect("open legacy database");
    connection
        .execute_batch(
            "
            PRAGMA foreign_keys = ON;
            CREATE TABLE cases (
                case_id TEXT PRIMARY KEY CHECK(case_id GLOB 'CASE-*'),
                campus_code TEXT NOT NULL CHECK(campus_code GLOB 'CAMPUS-*'),
                representative_count INTEGER NOT NULL DEFAULT 0 CHECK(representative_count BETWEEN 0 AND 2),
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE assets (
                asset_id TEXT PRIMARY KEY CHECK(asset_id GLOB 'ASSET-*'),
                case_id TEXT NOT NULL REFERENCES cases(case_id),
                kind TEXT NOT NULL CHECK(kind IN ('original_image', 'captured_frame')),
                width INTEGER NOT NULL CHECK(width >= 0),
                height INTEGER NOT NULL CHECK(height >= 0),
                source_fingerprint TEXT NOT NULL
            );
            CREATE TABLE representative_assets (
                case_id TEXT NOT NULL REFERENCES cases(case_id),
                asset_id TEXT NOT NULL REFERENCES assets(asset_id),
                selected_at INTEGER NOT NULL,
                PRIMARY KEY(case_id, asset_id)
            );
            CREATE TRIGGER representative_assets_limit_before_insert
            BEFORE INSERT ON representative_assets
            WHEN (SELECT COUNT(*) FROM representative_assets WHERE case_id = NEW.case_id) >= 2
            BEGIN
                SELECT RAISE(ABORT, 'representative_limit');
            END;
            CREATE TABLE templates (
                template_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                sha256 TEXT NOT NULL,
                canonical_json TEXT NOT NULL,
                PRIMARY KEY(template_id, version)
            );
            CREATE TABLE regions (
                region_id TEXT PRIMARY KEY CHECK(region_id GLOB 'REGION-*'),
                asset_id TEXT NOT NULL REFERENCES assets(asset_id),
                polygon_norm_json TEXT NOT NULL,
                template_id TEXT NOT NULL,
                template_version INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                FOREIGN KEY(template_id, template_version) REFERENCES templates(template_id, version)
            );
            CREATE TABLE region_label_values (
                region_id TEXT NOT NULL REFERENCES regions(region_id),
                field_id TEXT NOT NULL,
                value_json TEXT NOT NULL,
                PRIMARY KEY(region_id, field_id)
            );
            PRAGMA user_version = 1;
            ",
        )
        .expect("create legacy schema");
    connection
        .execute(
            "INSERT INTO cases (case_id, campus_code, representative_count, updated_at) VALUES (?1, ?2, 0, 1)",
            params![case.case_id, campus_code],
        )
        .expect("insert legacy case");
    for asset in assets {
        connection
            .execute(
                "INSERT INTO assets (asset_id, case_id, kind, width, height, source_fingerprint) VALUES (?1, ?2, 'original_image', 1, 1, 'legacy')",
                params![asset.asset_id, case.case_id],
            )
            .expect("insert legacy asset");
    }
    for (index, asset) in assets.iter().take(2).enumerate() {
        connection
            .execute(
                "INSERT INTO representative_assets (case_id, asset_id, selected_at) VALUES (?1, ?2, ?3)",
                params![case.case_id, asset.asset_id, index as i64 + 1],
            )
            .expect("insert legacy representative");
    }
    let template = generic_template();
    let snapshot = template.snapshot().expect("template snapshot");
    connection
        .execute(
            "INSERT INTO templates (template_id, version, sha256, canonical_json) VALUES (?1, ?2, ?3, ?4)",
            params![snapshot.template_id, snapshot.version, snapshot.sha256, snapshot.canonical_json],
        )
        .expect("insert legacy template");
    connection
        .execute(
            "INSERT INTO regions (region_id, asset_id, polygon_norm_json, template_id, template_version, created_at) VALUES ('REGION-LEGACY', ?1, '[{\"x\":0.1,\"y\":0.1},{\"x\":0.8,\"y\":0.1},{\"x\":0.4,\"y\":0.8}]', 'template-alpha', 1, 1)",
            params![&assets[0].asset_id],
        )
        .expect("insert legacy region");
    connection
        .execute(
            "INSERT INTO region_label_values (region_id, field_id, value_json) VALUES ('REGION-LEGACY', 'field_alpha', '\"option_alpha\"')",
            [],
        )
        .expect("insert legacy label");
}

#[test]
fn legacy_v1_database_migrates_without_limit_and_preserves_data_after_reopen() {
    let fixture = fixture(4);
    let discovery_work_root = fixture._temporary.path().join("DISCOVERY");
    let discovery = service_with_work_root(&discovery_work_root);
    let scan = discovery
        .scan_selected_root(&fixture.source_root)
        .expect("scan synthetic fixture for opaque IDs");
    let (case, _) = first_image(&scan);
    let assets = all_images(&scan)
        .into_iter()
        .filter_map(|(candidate_case, asset)| {
            (candidate_case.case_id == case.case_id).then_some(asset)
        })
        .collect::<Vec<_>>();
    create_legacy_v1_database(&fixture, &case, "CAMPUS-ALPHA", &assets);

    {
        let migrated = service(&fixture);
        migrated
            .scan_selected_root(&fixture.source_root)
            .expect("open and migrate legacy database");
        assert_eq!(
            migrated
                .representative_asset_ids(&case.case_id)
                .expect("preserved representative selections"),
            assets
                .iter()
                .take(2)
                .map(|asset| asset.asset_id.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            migrated
                .representative_count(&case.case_id)
                .expect("actual count"),
            2
        );
        assert_eq!(
            migrated
                .recorded_representative_count(&case.case_id)
                .expect("recomputed recorded count"),
            2
        );
        assert_eq!(
            migrated
                .saved_regions_for_asset(&case.case_id, &assets[0].asset_id)
                .expect("preserved regions")
                .iter()
                .map(|region| region.region_id.as_str())
                .collect::<Vec<_>>(),
            vec!["REGION-LEGACY"]
        );
        assert!(matches!(
            migrated.deselect_representative(&case.case_id, &assets[0].asset_id),
            Err(CoreError::RepresentativeHasRegions)
        ));
        assert_eq!(
            migrated
                .select_representative(&case.case_id, &assets[2].asset_id)
                .expect("select third representative after migration")
                .representative_count,
            3
        );
    }

    let connection = Connection::open(fixture.work_root.join("annotations.sqlite3"))
        .expect("inspect migrated database");
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("schema version");
    assert_eq!(version, 3);
    let cases_sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'cases'",
            [],
            |row| row.get(0),
        )
        .expect("new cases schema");
    assert!(!cases_sql.contains("BETWEEN 0 AND 2"));
    let legacy_trigger: Option<String> = connection
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'trigger' AND name = 'representative_assets_limit_before_insert'",
            [],
            |row| row.get(0),
        )
        .optional()
        .expect("legacy trigger lookup");
    assert_eq!(legacy_trigger, None);
    {
        let mut foreign_key_check = connection
            .prepare("PRAGMA foreign_key_check")
            .expect("foreign key check statement");
        let mut foreign_key_rows = foreign_key_check
            .query([])
            .expect("foreign key check query");
        assert!(foreign_key_rows
            .next()
            .expect("foreign key check row")
            .is_none());
    }
    drop(connection);

    let reopened = service(&fixture);
    reopened
        .scan_selected_root(&fixture.source_root)
        .expect("reopen migrated database");
    assert_eq!(
        reopened
            .select_representative(&case.case_id, &assets[3].asset_id)
            .expect("select fourth representative after reopen")
            .representative_count,
        4
    );
}

#[test]
fn saved_regions_block_representative_deselection() {
    let fixture = fixture(1);
    let service = service(&fixture);
    let scan = service
        .scan_selected_root(&fixture.source_root)
        .expect("scan fixture");
    let (case, asset) = first_image(&scan);
    load_generic_template(&service);
    service
        .select_representative(&case.case_id, &asset.asset_id)
        .expect("representative selection");
    service
        .save_confirmed_region(region_request(asset.asset_id.clone()))
        .expect("confirmed region save");

    let result = service.deselect_representative(&case.case_id, &asset.asset_id);
    assert!(matches!(result, Err(CoreError::RepresentativeHasRegions)));
    assert_eq!(
        service.representative_count(&case.case_id).expect("count"),
        1
    );
    assert_eq!(
        service
            .recorded_representative_count(&case.case_id)
            .expect("recorded count"),
        1
    );
    assert_eq!(service.region_count().expect("region count"), 1);
}

#[test]
fn local_ui_region_and_image_responses_are_deidentified_and_read_only() {
    let fixture = fixture(1);
    let source_before = source_tree_snapshot(&fixture.source_root);
    let service = service(&fixture);
    let scan = service
        .scan_selected_root(&fixture.source_root)
        .expect("scan fixture");
    let (case, asset) = first_image(&scan);
    load_generic_template(&service);
    service
        .select_representative(&case.case_id, &asset.asset_id)
        .expect("representative selection");
    service
        .save_confirmed_region(region_request(asset.asset_id.clone()))
        .expect("confirmed region save");

    let payload = service
        .read_image_asset(&case.case_id, &asset.asset_id)
        .expect("on-demand image payload");
    let representatives = service
        .representative_asset_ids(&case.case_id)
        .expect("representative IDs");
    let regions = service
        .saved_regions_for_asset(&case.case_id, &asset.asset_id)
        .expect("saved regions");
    let local_ui_json = serde_json::to_string(&(payload, representatives, regions))
        .expect("local UI response serialization");

    assert!(local_ui_json.contains(&asset.asset_id));
    assert!(!local_ui_json.contains(&case.case_id));
    assert!(!local_ui_json.contains("20240101_CASE-ALPHA"));
    assert!(!local_ui_json.contains("ASSET-01.JPG"));
    assert!(!local_ui_json.contains(fixture.source_root.to_string_lossy().as_ref()));
    assert_eq!(source_tree_snapshot(&fixture.source_root), source_before);
}

#[cfg(unix)]
#[test]
fn replaced_source_image_symlink_is_rejected_at_open_time() {
    use std::os::unix::fs::symlink;

    let fixture = fixture(1);
    let service = service(&fixture);
    let scan = service
        .scan_selected_root(&fixture.source_root)
        .expect("scan fixture");
    let (case, asset) = first_image(&scan);
    let original = fixture
        .source_root
        .join("CAMPUS-ALPHA")
        .join("20240101_CASE-ALPHA")
        .join("ASSET-01.JPG");
    let target = fixture._temporary.path().join("OUTSIDE-SOURCE-TARGET");
    fs::write(&target, b"must-not-be-returned").expect("target fixture");
    fs::remove_file(&original).expect("replace source fixture");
    symlink(&target, &original).expect("source replacement symlink");

    assert!(matches!(
        service.read_image_asset(&case.case_id, &asset.asset_id),
        Err(CoreError::UnsafeSourceAsset)
    ));
    assert_eq!(
        fs::read(&target).expect("target bytes"),
        b"must-not-be-returned"
    );
}

#[cfg(unix)]
#[test]
fn replaced_source_case_symlink_is_rejected_before_external_bytes_are_read() {
    use std::os::unix::fs::symlink;

    let fixture = fixture(1);
    let service = service(&fixture);
    let scan = service
        .scan_selected_root(&fixture.source_root)
        .expect("scan fixture");
    let (case, asset) = first_image(&scan);
    let original_case = fixture
        .source_root
        .join("CAMPUS-ALPHA")
        .join("20240101_CASE-ALPHA");
    let outside_case = fixture._temporary.path().join("OUTSIDE-CASE");
    fs::create_dir_all(&outside_case).expect("external case fixture");
    fs::write(outside_case.join("ASSET-01.JPG"), b"must-not-be-returned")
        .expect("external media fixture");
    fs::remove_dir_all(&original_case).expect("replace scanned case");
    symlink(&outside_case, &original_case).expect("case replacement symlink");

    assert!(matches!(
        service.read_image_asset(&case.case_id, &asset.asset_id),
        Err(CoreError::UnsafeSourceAsset)
    ));
}

#[cfg(unix)]
#[test]
fn replaced_source_image_with_same_name_is_rejected_at_open_time() {
    let fixture = fixture(1);
    let service = service(&fixture);
    let scan = service
        .scan_selected_root(&fixture.source_root)
        .expect("scan fixture");
    let (case, asset) = first_image(&scan);
    let original = fixture
        .source_root
        .join("CAMPUS-ALPHA")
        .join("20240101_CASE-ALPHA")
        .join("ASSET-01.JPG");
    fs::remove_file(&original).expect("replace source fixture");
    fs::write(&original, b"same-name-replacement").expect("replacement fixture");

    assert!(matches!(
        service.read_image_asset(&case.case_id, &asset.asset_id),
        Err(CoreError::UnsafeSourceAsset)
    ));
}

#[cfg(windows)]
#[test]
fn replaced_source_case_junction_is_rejected_before_asset_read() {
    use std::process::Command;

    let fixture = fixture(1);
    let service = service(&fixture);
    let scan = service
        .scan_selected_root(&fixture.source_root)
        .expect("scan fixture");
    let (case, asset) = first_image(&scan);
    let original_case = fixture
        .source_root
        .join("CAMPUS-ALPHA")
        .join("20240101_CASE-ALPHA");
    let outside_case = fixture._temporary.path().join("OUTSIDE-CASE");
    fs::create_dir_all(&outside_case).expect("external case fixture");
    fs::write(outside_case.join("ASSET-01.JPG"), b"must-not-be-returned")
        .expect("external media fixture");
    fs::remove_dir_all(&original_case).expect("replace scanned case");
    let status = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&original_case)
        .arg(&outside_case)
        .status()
        .expect("create junction");
    assert!(status.success(), "create source replacement junction");

    assert!(matches!(
        service.read_image_asset(&case.case_id, &asset.asset_id),
        Err(CoreError::UnsafeSourceAsset)
    ));
}

#[test]
fn label_txt_descriptor_returns_the_backend_validated_template_ids() {
    let fixture = fixture(1);
    let service = service(&fixture);
    let descriptor = service
        .import_label_txt("*Field Alpha=Option Alpha|Option Beta", "template-alpha", 1)
        .expect("label text import");

    assert_eq!(descriptor.template.template_id, descriptor.template_id);
    assert_eq!(descriptor.template.version, descriptor.version);
    assert!(descriptor.template.fields[0].id.starts_with("field-"));
    assert!(descriptor.template.fields[0].options[0]
        .id
        .starts_with("option-"));
}

#[test]
fn save_is_atomic() {
    let fixture = fixture(1);
    let service = service(&fixture);
    let scan = service
        .scan_selected_root(&fixture.source_root)
        .expect("scan fixture");
    let (case, asset) = first_image(&scan);
    load_generic_template(&service);
    service
        .select_representative(&case.case_id, &asset.asset_id)
        .expect("representative selection");

    let region = region_request(asset.asset_id);
    let failed =
        service.save_confirmed_region_with_fault(region.clone(), SaveFault::AfterRegionInsert);
    assert!(matches!(failed, Err(CoreError::FaultInjected)));
    assert_eq!(service.region_count().expect("region count"), 0);
    assert_eq!(service.region_label_count().expect("label count"), 0);

    service
        .save_confirmed_region(region)
        .expect("confirmed save");
    assert_eq!(service.region_count().expect("region count"), 1);
    assert_eq!(service.region_label_count().expect("label count"), 1);
}

#[test]
fn opaque_id_stability() {
    let fixture = fixture(1);
    let service = service(&fixture);
    let first = service
        .scan_selected_root(&fixture.source_root)
        .expect("initial scan");
    let (first_case, first_asset) = first_image(&first);
    let first_case_id = first_case.case_id;
    let first_id = first_asset.asset_id;
    let media = fixture
        .source_root
        .join("CAMPUS-ALPHA")
        .join("20240101_CASE-ALPHA")
        .join("ASSET-01.JPG");

    let stable = service
        .scan_selected_root(&fixture.source_root)
        .expect("repeat scan");
    let (stable_case, stable_asset) = first_image(&stable);
    assert_eq!(stable_case.case_id, first_case_id);
    assert_eq!(stable_asset.asset_id, first_id);

    let renamed = media.with_file_name("ASSET-02.JPG");
    fs::rename(&media, &renamed).expect("rename fixture media");
    let after_name = service
        .scan_selected_root(&fixture.source_root)
        .expect("scan after name change");
    let after_name_id = first_image(&after_name).1.asset_id;
    assert_ne!(after_name_id, first_id);

    fs::write(&renamed, b"fixture-with-new-size").expect("change fixture size");
    let after_size = service
        .scan_selected_root(&fixture.source_root)
        .expect("scan after size change");
    let after_size_id = first_image(&after_size).1.asset_id;
    assert_ne!(after_size_id, after_name_id);

    filetime::set_file_mtime(&renamed, FileTime::from_unix_time(1_600_000_000, 0))
        .expect("change fixture timestamp");
    let after_time = service
        .scan_selected_root(&fixture.source_root)
        .expect("scan after timestamp change");
    assert_ne!(first_image(&after_time).1.asset_id, after_size_id);
}

#[test]
fn privacy_leak_scan() {
    let fixture = fixture(1);
    let service = service(&fixture);
    let scan = service
        .scan_selected_root(&fixture.source_root)
        .expect("scan fixture");
    let (case, asset) = first_image(&scan);
    let local_display = service
        .current_local_display_scan()
        .expect("local display scan");
    let local_display_json = serde_json::to_string(&local_display).expect("local display JSON");
    assert!(local_display_json.contains("CAMPUS-ALPHA"));
    assert!(local_display_json.contains("20240101_CASE-ALPHA"));
    load_generic_template(&service);
    service
        .select_representative(&case.case_id, &asset.asset_id)
        .expect("representative selection");
    service
        .save_confirmed_region(region_request(asset.asset_id.clone()))
        .expect("save region");
    service
        .write_recovery_state(&RecoveryState {
            current_case_id: case.case_id.clone(),
            current_asset_id: asset.asset_id.clone(),
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
        })
        .expect("recovery state");
    service
        .write_thumbnail(&asset.asset_id, b"derived-thumbnail")
        .expect("thumbnail");
    service
        .write_capture(&case.case_id, &asset.asset_id, b"derived-capture")
        .expect("capture");

    let persisted_files = collect_file_bytes(service.work_root());
    let session_path = Path::new("session/project-session.json");
    let session_text = String::from_utf8_lossy(
        persisted_files
            .get(session_path)
            .expect("local-only project session"),
    );
    assert!(session_text.contains(fixture.source_root.to_string_lossy().as_ref()));
    assert!(!session_text.contains(&case.case_id));
    assert!(!session_text.contains("20240101_CASE-ALPHA"));
    assert!(!session_text.contains("ASSET-01.JPG"));

    let persisted_text = persisted_files
        .into_iter()
        .filter(|(path, _)| path != session_path)
        .map(|(_, bytes)| String::from_utf8_lossy(&bytes).into_owned())
        .collect::<String>();
    assert!(!persisted_text.contains("20240101_CASE-ALPHA"));
    assert!(!persisted_text.contains("CAMPUS-ALPHA"));
    assert!(!persisted_text.contains("ASSET-01.JPG"));
    let source_root_text = fixture.source_root.to_string_lossy();
    assert!(!persisted_text.contains(source_root_text.as_ref()));
    assert!(!persisted_text.contains("original_display_name"));
    assert!(!persisted_text.contains("source_path"));
}

#[test]
fn atomic_rewrites_replace_existing_derived_files() {
    let fixture = fixture(1);
    let service = service(&fixture);
    let scan = service
        .scan_selected_root(&fixture.source_root)
        .expect("scan fixture");
    let (case, asset) = first_image(&scan);
    let recovery_first = RecoveryState {
        current_case_id: case.case_id.clone(),
        current_asset_id: asset.asset_id.clone(),
        zoom: 1.0,
        pan_x: 0.0,
        pan_y: 0.0,
    };
    let recovery_second = RecoveryState {
        zoom: 2.0,
        pan_x: 3.0,
        pan_y: 4.0,
        ..recovery_first.clone()
    };

    let recovery_path = service
        .write_recovery_state(&recovery_first)
        .expect("first recovery write");
    let second_recovery_path = service
        .write_recovery_state(&recovery_second)
        .expect("second recovery write");
    assert_eq!(recovery_path, second_recovery_path);
    assert_eq!(
        fs::read(&recovery_path).expect("recovery bytes"),
        serde_json::to_vec(&recovery_second).expect("expected recovery bytes")
    );

    let thumbnail_path = service
        .write_thumbnail(&asset.asset_id, b"thumbnail-first")
        .expect("first thumbnail write");
    let second_thumbnail_path = service
        .write_thumbnail(&asset.asset_id, b"thumbnail-second")
        .expect("second thumbnail write");
    assert_eq!(thumbnail_path, second_thumbnail_path);
    assert_eq!(
        fs::read(&thumbnail_path).expect("thumbnail bytes"),
        b"thumbnail-second"
    );

    let capture_path = service
        .write_capture(&case.case_id, &asset.asset_id, b"capture-first")
        .expect("first capture write");
    let second_capture_path = service
        .write_capture(&case.case_id, &asset.asset_id, b"capture-second")
        .expect("second capture write");
    assert_eq!(capture_path, second_capture_path);
    assert_eq!(
        fs::read(&capture_path).expect("capture bytes"),
        b"capture-second"
    );
}

#[test]
fn derived_files_outside_source_root() {
    let fixture = fixture(1);
    let source_before = collect_file_bytes(&fixture.source_root);
    let service = service(&fixture);
    let scan = service
        .scan_selected_root(&fixture.source_root)
        .expect("scan fixture");
    let (case, asset) = first_image(&scan);

    service
        .write_thumbnail(&asset.asset_id, b"derived-thumbnail")
        .expect("thumbnail");
    service
        .write_capture(&case.case_id, &asset.asset_id, b"derived-capture")
        .expect("capture");
    service
        .write_recovery_state(&RecoveryState {
            current_case_id: case.case_id,
            current_asset_id: asset.asset_id,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
        })
        .expect("recovery state");

    assert_eq!(collect_file_bytes(&fixture.source_root), source_before);
    let work_files = collect_file_bytes(service.work_root());
    assert!(work_files
        .keys()
        .any(|path| path == Path::new("annotations.sqlite3")));
    assert!(work_files.keys().any(|path| path.starts_with("captures")));
    assert!(work_files.keys().any(|path| path.starts_with("thumbnails")));
    assert!(work_files.keys().any(|path| path.starts_with("recovery")));
}

#[cfg(unix)]
#[test]
fn nested_output_symlinks_are_rejected_without_touching_source_tree() {
    use std::os::unix::fs::symlink;

    let fixture = fixture(1);
    let source_before = source_tree_snapshot(&fixture.source_root);
    let service = service(&fixture);
    let scan = service
        .scan_selected_root(&fixture.source_root)
        .expect("scan fixture");
    let (case, asset) = first_image(&scan);

    symlink(&fixture.source_root, fixture.work_root.join("recovery"))
        .expect("recovery escape link");
    symlink(&fixture.source_root, fixture.work_root.join("thumbnails"))
        .expect("thumbnail escape link");
    symlink(&fixture.source_root, fixture.work_root.join("captures")).expect("capture escape link");

    let recovery = RecoveryState {
        current_case_id: case.case_id.clone(),
        current_asset_id: asset.asset_id.clone(),
        zoom: 1.0,
        pan_x: 0.0,
        pan_y: 0.0,
    };
    assert!(matches!(
        service.write_recovery_state(&recovery),
        Err(CoreError::UnsafeDerivedPath)
    ));
    assert!(matches!(
        service.write_thumbnail(&asset.asset_id, b"derived-thumbnail"),
        Err(CoreError::UnsafeDerivedPath)
    ));
    assert!(matches!(
        service.write_capture(&case.case_id, &asset.asset_id, b"derived-capture"),
        Err(CoreError::UnsafeDerivedPath)
    ));

    assert_eq!(source_tree_snapshot(&fixture.source_root), source_before);
}

#[cfg(unix)]
#[test]
fn database_artifact_symlinks_are_rejected_without_touching_source_tree() {
    use std::os::unix::fs::symlink;

    for database_file in [
        "annotations.sqlite3",
        "annotations.sqlite3-journal",
        "annotations.sqlite3-wal",
        "annotations.sqlite3-shm",
    ] {
        let fixture = fixture(1);
        let source_before = source_tree_snapshot(&fixture.source_root);
        fs::create_dir_all(&fixture.work_root).expect("work root");
        symlink(&fixture.source_root, fixture.work_root.join(database_file))
            .expect("database escape link");

        let service = service(&fixture);
        let result = service.scan_selected_root(&fixture.source_root);
        assert!(
            matches!(result, Err(CoreError::UnsafeDerivedPath)),
            "database artifact {database_file} must be rejected before SQLite opens it"
        );
        assert_eq!(source_tree_snapshot(&fixture.source_root), source_before);
    }
}

#[test]
fn database_artifact_non_regular_file_is_rejected_without_touching_source_tree() {
    let fixture = fixture(1);
    let source_before = source_tree_snapshot(&fixture.source_root);
    fs::create_dir_all(fixture.work_root.join("annotations.sqlite3-journal"))
        .expect("unsafe database directory");

    let service = service(&fixture);
    assert!(matches!(
        service.scan_selected_root(&fixture.source_root),
        Err(CoreError::UnsafeDerivedPath)
    ));
    assert_eq!(source_tree_snapshot(&fixture.source_root), source_before);
}

#[cfg(unix)]
#[test]
fn database_artifacts_are_rechecked_before_a_transaction() {
    use std::os::unix::fs::symlink;

    let fixture = fixture(1);
    let source_before = source_tree_snapshot(&fixture.source_root);
    let service = service(&fixture);
    let scan = service
        .scan_selected_root(&fixture.source_root)
        .expect("safe database initialization");
    let (case, asset) = first_image(&scan);

    let wal_path = fixture.work_root.join("annotations.sqlite3-wal");
    fs::remove_file(&wal_path).expect("remove safe WAL path");
    symlink(&fixture.source_root, &wal_path).expect("WAL escape link");

    assert!(matches!(
        service.select_representative(&case.case_id, &asset.asset_id),
        Err(CoreError::UnsafeDerivedPath)
    ));
    assert_eq!(source_tree_snapshot(&fixture.source_root), source_before);
}

#[test]
fn normal_database_artifacts_are_regular_files() {
    let fixture = fixture(1);
    let service = service(&fixture);
    service
        .scan_selected_root(&fixture.source_root)
        .expect("safe database initialization");

    for database_file in [
        "annotations.sqlite3",
        "annotations.sqlite3-journal",
        "annotations.sqlite3-wal",
        "annotations.sqlite3-shm",
    ] {
        let metadata =
            fs::symlink_metadata(fixture.work_root.join(database_file)).expect("database artifact");
        assert!(metadata.file_type().is_file());
        assert!(!metadata.file_type().is_symlink());
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SourceEntryKind {
    File,
    Directory,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceEntrySnapshot {
    kind: SourceEntryKind,
    size_bytes: u64,
    content_sha256: Option<Vec<u8>>,
    modified_millis: u128,
}

#[test]
fn source_media_is_unchanged_after_selection_and_region_save() {
    let fixture = fixture(1);
    let source_before = source_tree_snapshot(&fixture.source_root);
    let service = service(&fixture);
    let scan = service
        .scan_selected_root(&fixture.source_root)
        .expect("scan fixture");
    let (case, asset) = first_image(&scan);
    load_generic_template(&service);
    service
        .select_representative(&case.case_id, &asset.asset_id)
        .expect("representative selection");
    service
        .save_confirmed_region(region_request(asset.asset_id.clone()))
        .expect("confirmed region save");
    service
        .write_thumbnail(&asset.asset_id, b"derived-thumbnail")
        .expect("thumbnail write");
    service
        .write_capture(&case.case_id, &asset.asset_id, b"derived-capture")
        .expect("capture write");
    service
        .write_recovery_state(&RecoveryState {
            current_case_id: case.case_id,
            current_asset_id: asset.asset_id,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
        })
        .expect("recovery write");

    assert_eq!(source_tree_snapshot(&fixture.source_root), source_before);
}

#[test]
fn source_and_work_roots_reject_same_and_nested_paths_before_scan() {
    let temporary = tempfile::tempdir().expect("temporary fixture");

    let same_root = temporary.path().join("SHARED-ROOT");
    fs::create_dir_all(&same_root).expect("shared root");
    let same_before = source_tree_snapshot(&same_root);
    let same_service = service_with_work_root(&same_root);
    assert!(matches!(
        same_service.scan_selected_root(&same_root),
        Err(CoreError::SourceWorkOverlap)
    ));
    assert_eq!(source_tree_snapshot(&same_root), same_before);

    let source_parent = temporary.path().join("SOURCE-PARENT");
    fs::create_dir_all(&source_parent).expect("source parent");
    let child_work_root = source_parent.join("WORK-ROOT");
    let child_before = source_tree_snapshot(&source_parent);
    let child_service = service_with_work_root(&child_work_root);
    assert!(matches!(
        child_service.scan_selected_root(&source_parent),
        Err(CoreError::SourceWorkOverlap)
    ));
    assert_eq!(source_tree_snapshot(&source_parent), child_before);

    let work_parent = temporary.path().join("WORK-PARENT");
    let child_source_root = work_parent.join("SOURCE-ROOT");
    fs::create_dir_all(&child_source_root).expect("child source");
    let parent_before = source_tree_snapshot(&work_parent);
    let parent_service = service_with_work_root(&work_parent);
    assert!(matches!(
        parent_service.scan_selected_root(&child_source_root),
        Err(CoreError::SourceWorkOverlap)
    ));
    assert_eq!(source_tree_snapshot(&work_parent), parent_before);
}

#[cfg(unix)]
#[test]
fn resolved_symlink_overlap_is_rejected_before_scan_and_write() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().expect("temporary fixture");
    let unsafe_source = temporary.path().join("ROOT-CASE-FIXTURE");
    fs::create_dir_all(&unsafe_source).expect("source root");
    let unsafe_work_alias = temporary.path().join("WORK-ALIAS");
    symlink(&unsafe_source, &unsafe_work_alias).expect("work alias");
    let unsafe_before = source_tree_snapshot(&unsafe_source);
    let unsafe_service = service_with_work_root(&unsafe_work_alias);
    assert!(matches!(
        unsafe_service.scan_selected_root(&unsafe_source),
        Err(CoreError::SourceWorkOverlap)
    ));
    assert_eq!(source_tree_snapshot(&unsafe_source), unsafe_before);

    let fixture = fixture(1);
    let source_alias = fixture._temporary.path().join("SOURCE-ALIAS");
    symlink(&fixture.source_root, &source_alias).expect("source symlink");
    let service = service(&fixture);
    let scan = service
        .scan_selected_root(&source_alias)
        .expect("scan through safe symlink");
    let (case, asset) = first_image(&scan);

    fs::remove_file(&source_alias).expect("remove source symlink");
    symlink(&fixture.work_root, &source_alias).expect("overlapping symlink");
    assert!(matches!(
        service.select_representative(&case.case_id, &asset.asset_id),
        Err(CoreError::SourceWorkOverlap)
    ));
}

#[test]
fn fixed_toml_template_validates_labels_and_reloads_with_the_saved_session() {
    let fixture = fixture(1);
    let source_before = source_tree_snapshot(&fixture.source_root);
    let (case, asset) = {
        let service = service(&fixture);
        let scan = service
            .scan_selected_root(&fixture.source_root)
            .expect("scan fixture");
        let (case, asset) = first_image(&scan);
        let descriptor = service
            .install_fixed_toml_template(V02_TOML_TEMPLATE)
            .expect("install first fixed TOML template");
        assert_eq!(descriptor.template_id, "template-v02");
        assert_eq!(descriptor.template.fields.len(), 3);
        assert_eq!(
            descriptor.template.fields[0].control,
            FieldControl::Dropdown
        );
        assert_eq!(
            descriptor.template.fields[1].control,
            FieldControl::SingleSelect
        );
        assert_eq!(
            descriptor.template.fields[2].control,
            FieldControl::MultiSelect
        );
        assert_eq!(
            service
                .current_fixed_template()
                .expect("current fixed template")
                .expect("fixed template installed"),
            descriptor
        );
        service
            .select_representative(&case.case_id, &asset.asset_id)
            .expect("select representative");
        assert!(matches!(
            service.save_confirmed_region(v02_region_request(
                "REGION-TOMLTEST",
                asset.asset_id.clone(),
                BTreeMap::new(),
            )),
            Err(CoreError::InvalidLabels)
        ));
        service
            .save_confirmed_region(v02_region_request(
                "REGION-TOMLTEST",
                asset.asset_id.clone(),
                v02_labels("dropdown_a"),
            ))
            .expect("save valid TOML-backed region");
        service
            .save_last_position(&case.case_id, &asset.asset_id)
            .expect("save current position");
        assert_eq!(
            fs::read_to_string(
                fixture
                    .work_root
                    .join("templates")
                    .join("label-template-v1.toml"),
            )
            .expect("fixed TOML copy"),
            V02_TOML_TEMPLATE
        );
        assert!(fixture
            .work_root
            .join("templates")
            .join("label-template-v1.manifest.json")
            .is_file());
        assert!(matches!(
            service.install_fixed_toml_template(
                &V02_TOML_TEMPLATE.replace("template-v02", "template-other",)
            ),
            Err(CoreError::FixedTemplateAlreadyInstalled)
        ));
        (case, asset)
    };

    let reopened = service(&fixture);
    let restored = reopened
        .restore_project_session()
        .expect("restore fixed-template session");
    assert_eq!(restored.status, RestoreStatus::Restored);
    assert_eq!(
        restored.current_case_id.as_deref(),
        Some(case.case_id.as_str())
    );
    assert_eq!(
        restored.current_asset_id.as_deref(),
        Some(asset.asset_id.as_str())
    );
    assert_eq!(
        restored.template.expect("restored template").template_id,
        "template-v02"
    );
    assert_eq!(source_tree_snapshot(&fixture.source_root), source_before);
}

#[test]
fn project_session_handles_forget_missing_media_missing_template_and_missing_root() {
    let fixture = fixture(1);
    let (case, asset) = {
        let service = service(&fixture);
        let scan = service
            .scan_selected_root(&fixture.source_root)
            .expect("scan fixture");
        let (case, asset) = first_image(&scan);
        service
            .install_fixed_toml_template(V02_TOML_TEMPLATE)
            .expect("install fixed template");
        service
            .save_last_position(&case.case_id, &asset.asset_id)
            .expect("save position");
        (case, asset)
    };

    let restored = service(&fixture)
        .restore_project_session()
        .expect("restore session");
    assert_eq!(restored.status, RestoreStatus::Restored);
    assert_eq!(
        restored.current_case_id.as_deref(),
        Some(case.case_id.as_str())
    );
    assert_eq!(
        restored.current_asset_id.as_deref(),
        Some(asset.asset_id.as_str())
    );

    let forgetting_service = service(&fixture);
    forgetting_service
        .forget_last_position()
        .expect("forget saved session");
    assert_eq!(
        service(&fixture)
            .restore_project_session()
            .expect("empty session restore")
            .status,
        RestoreStatus::NoSession
    );

    let rescan_service = service(&fixture);
    rescan_service
        .scan_selected_root(&fixture.source_root)
        .expect("rescan fixture");
    rescan_service
        .install_fixed_toml_template(V02_TOML_TEMPLATE)
        .expect("identical fixed template remains valid");
    rescan_service
        .save_last_position(&case.case_id, &asset.asset_id)
        .expect("save position before media removal");
    drop(rescan_service);

    fs::remove_file(
        fixture
            .source_root
            .join("CAMPUS-ALPHA")
            .join("20240101_CASE-ALPHA")
            .join("ASSET-01.JPG"),
    )
    .expect("remove synthetic current asset");
    assert_eq!(
        service(&fixture)
            .restore_project_session()
            .expect("restore after current asset disappears")
            .status,
        RestoreStatus::LastMediaUnavailable
    );

    fs::remove_file(
        fixture
            .work_root
            .join("templates")
            .join("label-template-v1.toml"),
    )
    .expect("remove fixed TOML copy");
    assert_eq!(
        service(&fixture)
            .restore_project_session()
            .expect("restore with missing fixed template")
            .status,
        RestoreStatus::FixedTemplateMissing
    );

    fs::remove_dir_all(&fixture.source_root).expect("remove synthetic root");
    assert_eq!(
        service(&fixture)
            .restore_project_session()
            .expect("restore after synthetic root removal")
            .status,
        RestoreStatus::SavedRootUnavailable
    );
}

#[test]
fn region_update_replace_delete_keep_only_active_regions_and_audit_history() {
    let fixture = fixture(1);
    let (case, asset) = {
        let service = service(&fixture);
        let scan = service
            .scan_selected_root(&fixture.source_root)
            .expect("scan fixture");
        let (case, asset) = first_image(&scan);
        service
            .install_fixed_toml_template(V02_TOML_TEMPLATE)
            .expect("install template");
        service
            .select_representative(&case.case_id, &asset.asset_id)
            .expect("select representative");
        service
            .save_confirmed_region(v02_region_request(
                "REGION-ORIGINAL",
                asset.asset_id.clone(),
                v02_labels("dropdown_a"),
            ))
            .expect("save original region");
        service
            .update_region_labels(UpdateRegionLabelsRequest {
                case_id: case.case_id.clone(),
                asset_id: asset.asset_id.clone(),
                region_id: "REGION-ORIGINAL".to_owned(),
                template_id: "template-v02".to_owned(),
                template_version: 1,
                labels: v02_labels("dropdown_b"),
            })
            .expect("update region labels");
        assert_eq!(
            service
                .saved_regions_for_asset(&case.case_id, &asset.asset_id)
                .expect("updated active region")[0]
                .labels,
            v02_labels("dropdown_b")
        );
        service
            .replace_region(ReplaceRegionRequest {
                case_id: case.case_id.clone(),
                old_region_id: "REGION-ORIGINAL".to_owned(),
                replacement: v02_region_request(
                    "REGION-REPLACED",
                    asset.asset_id.clone(),
                    v02_labels("dropdown_a"),
                ),
            })
            .expect("replace region with redraw");
        let active = service
            .saved_regions_for_asset(&case.case_id, &asset.asset_id)
            .expect("replacement active region");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].region_id, "REGION-REPLACED");
        service
            .delete_region(&case.case_id, &asset.asset_id, "REGION-REPLACED")
            .expect("delete replacement region");
        assert!(service
            .saved_regions_for_asset(&case.case_id, &asset.asset_id)
            .expect("no active regions")
            .is_empty());
        assert_eq!(service.region_count().expect("active region count"), 0);
        assert_eq!(service.region_label_count().expect("active label count"), 0);
        service
            .deselect_representative(&case.case_id, &asset.asset_id)
            .expect("deleted regions no longer block deselection");
        (case, asset)
    };

    let connection = Connection::open(fixture.work_root.join("annotations.sqlite3"))
        .expect("inspect region audit data");
    let original: (String, Option<String>) = connection
        .query_row(
            "SELECT status, replacement_region_id FROM regions WHERE region_id = 'REGION-ORIGINAL'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("replaced source region");
    assert_eq!(
        original,
        ("replaced".to_owned(), Some("REGION-REPLACED".to_owned()))
    );
    let replacement_status: String = connection
        .query_row(
            "SELECT status FROM regions WHERE region_id = 'REGION-REPLACED'",
            [],
            |row| row.get(0),
        )
        .expect("deleted replacement region");
    assert_eq!(replacement_status, "deleted");
    let operations = connection
        .prepare("SELECT operation_kind FROM region_operations ORDER BY operation_id")
        .expect("region operation query")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("region operation rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("region operations");
    assert_eq!(
        operations,
        vec![
            "created",
            "labels_updated",
            "created",
            "replaced",
            "deleted"
        ]
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM regions WHERE asset_id = ?1 AND status = 'active'",
                params![asset.asset_id],
                |row| row.get::<_, i64>(0),
            )
            .expect("active region database count"),
        0
    );
    assert!(!case.case_id.is_empty());
}

#[test]
fn captured_frames_persist_as_annotatable_images_without_changing_the_source_tree() {
    let fixture = fixture(1);
    let source_before = source_tree_snapshot(&fixture.source_root);
    let (case, video, captured_asset_id) = {
        let service = service(&fixture);
        let scan = service
            .scan_selected_root(&fixture.source_root)
            .expect("scan fixture");
        let (case, video) = first_video(&scan);
        service
            .install_fixed_toml_template(V02_TOML_TEMPLATE)
            .expect("install template");
        assert!(matches!(
            service.capture_video_frame(CaptureFrameRequest {
                case_id: case.case_id.clone(),
                video_asset_id: video.asset_id.clone(),
                capture_time_millis: 0,
                width: 2,
                height: 2,
                png_base64: BASE64_STANDARD.encode(b"not-a-png"),
            }),
            Err(CoreError::InvalidCapture)
        ));
        assert!(matches!(
            service.capture_video_frame(CaptureFrameRequest {
                case_id: case.case_id.clone(),
                video_asset_id: video.asset_id.clone(),
                capture_time_millis: 0,
                width: 1,
                height: 2,
                png_base64: BASE64_STANDARD.encode(synthetic_png(2, 2, b"mismatched-dimensions")),
            }),
            Err(CoreError::InvalidCapture)
        ));
        let capture_bytes = synthetic_png(2, 2, b"synthetic-png-frame");
        assert_eq!(
            service
                .video_asset_bytes(&case.case_id, &video.asset_id)
                .expect("read synthetic video by opaque ID"),
            b"fixture-video"
        );
        let captured = service
            .capture_video_frame(CaptureFrameRequest {
                case_id: case.case_id.clone(),
                video_asset_id: video.asset_id.clone(),
                capture_time_millis: 1_234,
                width: 2,
                height: 2,
                png_base64: BASE64_STANDARD.encode(&capture_bytes),
            })
            .expect("persist captured frame");
        assert_eq!(captured.case_id, case.case_id);
        assert_eq!(captured.parent_video_asset_id, video.asset_id);
        assert_eq!(captured.capture_time_millis, 1_234);
        assert_eq!(captured.width, 2);
        assert_eq!(captured.height, 2);
        assert!(service
            .current_scan()
            .expect("scan with captured frame")
            .campuses
            .iter()
            .flat_map(|campus| campus.cases.iter())
            .find(|candidate| candidate.case_id == case.case_id)
            .expect("captured case")
            .assets
            .iter()
            .any(
                |asset| asset.asset_id == captured.asset_id && asset.media_kind == MediaKind::Image
            ));
        assert_eq!(
            BASE64_STANDARD
                .decode(
                    service
                        .read_image_asset(&case.case_id, &captured.asset_id)
                        .expect("captured image payload")
                        .data_base64,
                )
                .expect("captured image base64"),
            capture_bytes
        );
        service
            .select_representative(&case.case_id, &captured.asset_id)
            .expect("captured frame as representative");
        service
            .save_confirmed_region(v02_region_request(
                "REGION-CAPTURED",
                captured.asset_id.clone(),
                v02_labels("dropdown_a"),
            ))
            .expect("annotate captured frame");
        service
            .save_last_position(&case.case_id, &captured.asset_id)
            .expect("save captured frame position");
        (case, video, captured.asset_id)
    };

    let connection = Connection::open(fixture.work_root.join("annotations.sqlite3"))
        .expect("inspect capture source");
    let capture_source: (String, i64, u32, u32) = connection
        .query_row(
            "SELECT parent_video_asset_id, capture_time_millis, assets.width, assets.height
             FROM capture_sources INNER JOIN assets ON assets.asset_id = capture_sources.captured_asset_id
             WHERE captured_asset_id = ?1",
            params![captured_asset_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("capture source metadata");
    assert_eq!(capture_source, (video.asset_id.clone(), 1_234, 2, 2));
    drop(connection);

    let reopened = service(&fixture);
    let restored = reopened
        .restore_project_session()
        .expect("restore captured frame session");
    assert_eq!(restored.status, RestoreStatus::Restored);
    assert_eq!(
        restored.current_case_id.as_deref(),
        Some(case.case_id.as_str())
    );
    assert_eq!(
        restored.current_asset_id.as_deref(),
        Some(captured_asset_id.as_str())
    );
    assert!(restored
        .scan
        .expect("restored scan")
        .campuses
        .iter()
        .flat_map(|campus| campus.cases.iter())
        .find(|candidate| candidate.case_id == case.case_id)
        .expect("restored captured case")
        .assets
        .iter()
        .any(|asset| asset.asset_id == captured_asset_id && asset.media_kind == MediaKind::Image));
    assert_eq!(
        reopened
            .saved_regions_for_asset(&case.case_id, &captured_asset_id)
            .expect("captured annotations after restore")
            .len(),
        1
    );
    assert_eq!(source_tree_snapshot(&fixture.source_root), source_before);
}

#[test]
fn decoded_image_dimensions_are_validated_and_preserved_across_representative_reopen() {
    let fixture = fixture(1);
    let source_before = source_tree_snapshot(&fixture.source_root);
    let (case, asset) = {
        let service = service(&fixture);
        let scan = service
            .scan_selected_root(&fixture.source_root)
            .expect("scan fixture");
        let (case, asset) = first_image(&scan);
        assert!(matches!(
            service.record_image_dimensions(&case.case_id, &asset.asset_id, 0, 480),
            Err(CoreError::InvalidAssetDimensions)
        ));
        assert!(matches!(
            service.record_image_dimensions(&case.case_id, &asset.asset_id, 640, 0),
            Err(CoreError::InvalidAssetDimensions)
        ));
        service
            .record_image_dimensions(&case.case_id, &asset.asset_id, 640, 480)
            .expect("record decoded dimensions");
        service
            .select_representative(&case.case_id, &asset.asset_id)
            .expect("representative upsert must not reset known dimensions");
        (case, asset)
    };

    let reopened = service(&fixture);
    reopened
        .scan_selected_root(&fixture.source_root)
        .expect("reopen image scan");
    reopened
        .select_representative(&case.case_id, &asset.asset_id)
        .expect("repeat representative selection after reopen");
    drop(reopened);

    let connection = Connection::open(fixture.work_root.join("annotations.sqlite3"))
        .expect("inspect persisted dimensions");
    let dimensions: (u32, u32) = connection
        .query_row(
            "SELECT width, height FROM assets WHERE asset_id = ?1",
            params![asset.asset_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("persisted dimensions");
    assert_eq!(dimensions, (640, 480));
    assert_eq!(source_tree_snapshot(&fixture.source_root), source_before);
}

#[test]
fn completed_case_backup_uses_a_consistent_sqlite_snapshot_and_includes_captures() {
    let fixture = fixture(1);
    let source_before = source_tree_snapshot(&fixture.source_root);
    let (case, captured_asset_id, backup, capture_bytes) = {
        let service = service(&fixture);
        let scan = service
            .scan_selected_root(&fixture.source_root)
            .expect("scan fixture");
        let (case, video) = first_video(&scan);
        service
            .install_fixed_toml_template(V02_TOML_TEMPLATE)
            .expect("install template");
        let capture_bytes = synthetic_png(3, 2, b"backup-capture-frame");
        let captured = service
            .capture_video_frame(CaptureFrameRequest {
                case_id: case.case_id.clone(),
                video_asset_id: video.asset_id,
                capture_time_millis: 2_468,
                width: 3,
                height: 2,
                png_base64: BASE64_STANDARD.encode(&capture_bytes),
            })
            .expect("capture frame for backup");
        service
            .select_representative(&case.case_id, &captured.asset_id)
            .expect("select captured representative");
        service
            .save_confirmed_region(v02_region_request(
                "REGION-BACKUPAA",
                captured.asset_id.clone(),
                v02_labels("dropdown_b"),
            ))
            .expect("commit annotation before backup");
        let backup = service
            .complete_case_and_backup(&case.case_id)
            .expect("complete case backup");
        assert_eq!(backup.case_id, case.case_id);
        assert_eq!(backup.capture_count, 1);
        (case, captured.asset_id, backup, capture_bytes)
    };

    let backup_root = fixture.work_root.join("backups").join(&case.case_id);
    let manifests = collect_file_bytes(&backup_root)
        .into_iter()
        .filter(|(path, _)| path.file_name().is_some_and(|name| name == "manifest.json"))
        .collect::<Vec<_>>();
    assert_eq!(manifests.len(), 1, "only one completed backup manifest");
    let (manifest_relative, manifest_bytes) = &manifests[0];
    let manifest: serde_json::Value =
        serde_json::from_slice(manifest_bytes).expect("backup completion manifest");
    assert_eq!(manifest["format"], "gwp-case-backup");
    assert_eq!(manifest["case_id"], case.case_id);
    assert_eq!(
        manifest["capture_asset_ids"],
        serde_json::json!([captured_asset_id.clone()])
    );
    assert_eq!(manifest["completed_at"], backup.completed_at);
    let backup_directory = manifest_relative.parent().expect("backup directory");
    let backup_database = backup_root
        .join(backup_directory)
        .join("annotations.sqlite3");
    let backup_connection = Connection::open(&backup_database).expect("open SQLite backup");
    assert_eq!(
        backup_connection
            .query_row(
                "SELECT COUNT(*) FROM regions WHERE asset_id = ?1 AND status = 'active'",
                params![captured_asset_id],
                |row| row.get::<_, i64>(0),
            )
            .expect("committed annotation in backup"),
        1
    );
    assert_eq!(
        backup_connection
            .query_row(
                "SELECT COUNT(*) FROM capture_sources WHERE captured_asset_id = ?1",
                params![captured_asset_id],
                |row| row.get::<_, i64>(0),
            )
            .expect("capture source in backup"),
        1
    );
    assert_eq!(
        fs::read(
            backup_root
                .join(backup_directory)
                .join("captures")
                .join(format!("FRAME-{captured_asset_id}.png")),
        )
        .expect("backup capture bytes"),
        capture_bytes
    );
    assert_eq!(
        fs::read_to_string(
            backup_root
                .join(backup_directory)
                .join("label-template-v1.toml"),
        )
        .expect("backup fixed template"),
        V02_TOML_TEMPLATE
    );
    assert_eq!(source_tree_snapshot(&fixture.source_root), source_before);
}

#[test]
fn template_snapshot_roundtrip() {
    let fixture = fixture(1);
    let service = service(&fixture);
    let scan = service
        .scan_selected_root(&fixture.source_root)
        .expect("scan fixture");
    let (case, asset) = first_image(&scan);
    load_generic_template(&service);
    service
        .select_representative(&case.case_id, &asset.asset_id)
        .expect("representative selection");
    service
        .save_confirmed_region(region_request(asset.asset_id))
        .expect("save region");

    let snapshot = service
        .template_snapshot_json("template-alpha", 1)
        .expect("snapshot query")
        .expect("snapshot");
    let parsed = LabelTemplate::parse_json(&snapshot).expect("snapshot JSON");
    assert_eq!(parsed.fields[0].id, "field_alpha");
}

fn collect_file_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    collect_file_bytes_into(root, root, &mut files);
    files
}

fn source_tree_snapshot(root: &Path) -> BTreeMap<PathBuf, SourceEntrySnapshot> {
    let mut snapshot = BTreeMap::new();
    source_tree_snapshot_into(root, root, &mut snapshot);
    snapshot
}

fn source_tree_snapshot_into(
    root: &Path,
    current: &Path,
    snapshot: &mut BTreeMap<PathBuf, SourceEntrySnapshot>,
) {
    let metadata = fs::symlink_metadata(current).expect("fixture metadata");
    let file_type = metadata.file_type();
    let kind = if file_type.is_file() {
        SourceEntryKind::File
    } else if file_type.is_dir() {
        SourceEntryKind::Directory
    } else {
        SourceEntryKind::Other
    };
    let relative = current
        .strip_prefix(root)
        .expect("relative fixture path")
        .to_path_buf();
    let content_sha256 = if file_type.is_file() {
        Some(Sha256::digest(fs::read(current).expect("fixture bytes")).to_vec())
    } else {
        None
    };
    let modified_millis = metadata
        .modified()
        .expect("fixture mtime")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("fixture mtime after epoch")
        .as_millis();
    snapshot.insert(
        relative,
        SourceEntrySnapshot {
            kind: kind.clone(),
            size_bytes: metadata.len(),
            content_sha256,
            modified_millis,
        },
    );

    if kind == SourceEntryKind::Directory {
        for entry in fs::read_dir(current).expect("fixture directory") {
            source_tree_snapshot_into(root, &entry.expect("fixture entry").path(), snapshot);
        }
    }
}

fn collect_file_bytes_into(root: &Path, current: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
    for entry in fs::read_dir(current).expect("fixture directory") {
        let entry = entry.expect("fixture entry");
        let path = entry.path();
        if entry.file_type().expect("fixture type").is_dir() {
            collect_file_bytes_into(root, &path, files);
        } else {
            let relative = path
                .strip_prefix(root)
                .expect("relative fixture path")
                .to_path_buf();
            files.insert(relative, fs::read(path).expect("fixture file"));
        }
    }
}

#[test]
fn fixture_file_names_are_deidentified_placeholders() {
    let fixture = fixture(1);
    let file_map = collect_file_bytes(&fixture.source_root);
    let names = file_map
        .keys()
        .flat_map(|path| path.components())
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<BTreeSet<_>>();
    assert!(names.iter().all(|name| {
        name.contains("CASE") || name.contains("ASSET") || name.contains("CAMPUS")
    }));
}
