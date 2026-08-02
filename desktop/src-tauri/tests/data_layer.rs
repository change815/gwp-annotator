use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use filetime::FileTime;
use gwp_annotator_core::core::error::CoreError;
use gwp_annotator_core::core::scan::{MediaKind, PublicAsset, PublicCase, PublicScan};
use gwp_annotator_core::core::service::SaveRegionRequest;
use gwp_annotator_core::core::store::{PointNorm, RecoveryState, SaveFault};
use gwp_annotator_core::core::template::{
    FieldControl, LabelTemplate, LabelValue, TemplateField, TemplateOption, LABEL_TEMPLATE_FORMAT,
};
use gwp_annotator_core::{FixedSecretProvider, ProjectService};
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
fn representative_limit_and_gate() {
    let fixture = fixture(3);
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

    for (_, asset) in images.iter().take(2) {
        service
            .select_representative(&case.case_id, &asset.asset_id)
            .expect("representative selection");
    }
    let rejected_third = service.select_representative(&case.case_id, &images[2].1.asset_id);
    assert!(matches!(
        rejected_third,
        Err(CoreError::RepresentativeLimit)
    ));
    assert_eq!(
        service.representative_count(&case.case_id).expect("count"),
        2
    );
    assert_eq!(
        service
            .recorded_representative_count(&case.case_id)
            .expect("recorded count"),
        2
    );
    let removed = service
        .deselect_representative(&case.case_id, &images[0].1.asset_id)
        .expect("cancel representative selection");
    assert_eq!(removed.representative_count, 1);
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
    let third = service
        .select_representative(&case.case_id, &images[2].1.asset_id)
        .expect("selection after cancellation");
    assert_eq!(third.representative_count, 2);
    assert_eq!(
        service
            .recorded_representative_count(&case.case_id)
            .expect("recorded count"),
        2
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

    let persisted_text = collect_file_bytes(service.work_root())
        .into_values()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
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
