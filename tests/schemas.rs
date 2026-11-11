//! The published copy of the tool schemas, schemas/tools.json, must match the
//! code, so schema changes show up in code review (docs/tool-api.md,
//! "Published copy"). To update it after a deliberate change, run
//! scripts/update-schemas.sh.

const PUBLISHED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schemas/tools.json");

#[test]
fn published_schemas_match_the_code() {
    let tools = logix_engine::tools::all_definitions();
    let current = serde_json::to_string_pretty(&tools).unwrap() + "\n";
    if std::env::var_os("UPDATE_SCHEMAS").is_some() {
        std::fs::write(PUBLISHED, &current).unwrap();
        return;
    }
    let published = std::fs::read_to_string(PUBLISHED).unwrap_or_default();
    assert!(
        published == current,
        "schemas/tools.json is out of date. Run scripts/update-schemas.sh and review the change. \
         Once released, changes may only add tools or optional fields."
    );
}

/// The board files the engine builds in follow Logix Catalog's schema, which
/// that repo owns and the website publishes, and every one reads as a custom
/// board, unchanged: a board copies between the catalog and a host's
/// custom boards folder as a file.
#[test]
fn the_catalog_follows_logix_catalogs_schema() {
    use logix_engine::catalog;
    let folder = catalog::test_copy_folder();
    let schema_text = std::fs::read_to_string(folder.join("schemas/logix-board-v1.json")).unwrap();
    let schema: serde_json::Value = serde_json::from_str(&schema_text).unwrap();
    let board_files = catalog::json_files(&folder.join("boards")).unwrap();
    catalog::use_test_copy();
    assert_eq!(schema["$id"], catalog::BOARDS_SCHEMA);
    let validator = jsonschema::validator_for(&schema).expect("a valid schema");
    assert_eq!(board_files.len(), catalog::get().boards.len());
    for (id, text) in &board_files {
        let file: serde_json::Value = serde_json::from_str(text).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&file)
            .map(|e| format!("{}: {e}", e.instance_path()))
            .collect();
        assert_eq!(errors, Vec::<String>::new(), "{id}");
        serde_json::from_value::<logix_engine::board::CustomBoard>(file)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    // The schema catches a typo the engine also refuses.
    let mut bad: serde_json::Value = serde_json::from_str(&board_files[0].1).unwrap();
    bad["nmae"] = "typo".into();
    assert!(validator.iter_errors(&bad).next().is_some());
    assert!(serde_json::from_value::<logix_engine::board::CustomBoard>(bad).is_err());
}
