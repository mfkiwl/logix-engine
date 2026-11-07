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

/// The boards schema's published copy. The website serves the same file at
/// its address (catalog::BOARDS_SCHEMA): copy it there after a change.
const BOARDS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/schemas/logix-boards-v1.json");

#[test]
fn published_boards_schema_matches_the_code() {
    let current =
        serde_json::to_string_pretty(&logix_engine::catalog::boards_schema()).unwrap() + "\n";
    if std::env::var_os("UPDATE_SCHEMAS").is_some() {
        std::fs::write(BOARDS, &current).unwrap();
        return;
    }
    let published = std::fs::read_to_string(BOARDS).unwrap_or_default();
    assert!(
        published == current,
        "schemas/logix-boards-v1.json is out of date. Run scripts/update-schemas.sh, review the \
         change and copy the file to the website (docs/schemas/ in openlogix.github.io)."
    );
}

fn boards_validator() -> jsonschema::Validator {
    jsonschema::validator_for(&logix_engine::catalog::boards_schema()).expect("a valid schema")
}

fn schema_errors(file: &serde_json::Value) -> Vec<String> {
    boards_validator()
        .iter_errors(file)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect()
}

/// The catalog's own file follows the schema, so a catalog board can be
/// copied into a custom boards file as is, and back.
#[test]
fn the_catalog_follows_the_boards_schema() {
    let file: serde_json::Value =
        serde_json::from_str(include_str!("../catalog/boards.json")).unwrap();
    assert_eq!(file["$schema"], logix_engine::catalog::BOARDS_SCHEMA);
    assert_eq!(schema_errors(&file), Vec::<String>::new());
    // And every catalog board reads as a custom board, unchanged.
    let mut boards = file.as_object().unwrap().clone();
    boards.remove("$schema");
    for (id, board) in boards {
        serde_json::from_value::<logix_engine::board::CustomBoard>(board)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
    }
}

#[test]
fn the_boards_schema_catches_mistakes() {
    let good = serde_json::json!({
        "$schema": logix_engine::catalog::BOARDS_SCHEMA,
        "my-board": {
            "description": "My board",
            "fpga_id": "ice40up5k-sg48",
            "variants": [{
                "selector": {"connection": "usb", "memory": "flash"},
                "programmer": "iceprog", "args": "${BITSTREAM}",
                "usb": {"vid": "0403", "pid": "6014"}, "tested": false,
                "user_notes": "Untried.", "maintainer_notes": "From the vendor's page."
            }]
        }
    });
    assert_eq!(schema_errors(&good), Vec::<String>::new());
    let bad = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut v = good.clone();
        edit(&mut v);
        assert!(!schema_errors(&v).is_empty(), "{v}");
    };
    bad(&|v| v["$schema"] = "https://example.com/other.json".into());
    bad(&|v| v["my-board"]["descripton"] = "typo".into());
    bad(&|v| v["my-board"]["variants"][0]["usb"]["vid"] = "04G3".into());
    bad(&|v| v["my-board"]["variants"] = serde_json::json!([]));
    bad(&|v| {
        let b = v["my-board"].clone();
        v["My Board"] = b;
    });
}
