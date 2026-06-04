use std::collections::BTreeSet;
use std::fs;

use randdb_core::{index_repository, CoreRequest, CoreResponse, Store};
use serde::Deserialize;
use serde_json::{json, Value};
use tempfile::TempDir;

const TEXT_RESPONSE_ENVELOPE: &str =
    include_str!("../../../fixtures/golden/mcp/text-response-envelope.json");
const CODEBASE_RETRIEVAL_REQUEST: &str =
    include_str!("../../../fixtures/golden/mcp/codebase-retrieval-request.json");
const INDEXING_LIFECYCLE_EXPECTED: &str =
    include_str!("../../../fixtures/golden/indexing/lifecycle-expected.json");
const MULTIBYTE_PACKED_SEGMENT_RESPONSE: &str =
    include_str!("../../../fixtures/golden/multibyte/packed-segment-response.json");

#[derive(Debug, Deserialize)]
struct LifecycleExpected {
    first: IndexRunExpected,
    second: IndexRunExpected,
    third: IndexRunExpected,
}

#[derive(Debug, Deserialize)]
struct IndexRunExpected {
    scanned_files: u64,
    added_files: u64,
    modified_files: u64,
    deleted_files: u64,
    unchanged_files: u64,
    skipped_embeddings: u64,
    file_count: u64,
}

#[test]
fn golden_mcp_text_response_envelope_stays_contextweaver_compatible() {
    let value: Value = serde_json::from_str(TEXT_RESPONSE_ENVELOPE).expect("golden JSON parses");

    assert_eq!(
        value,
        json!({
            "content": [
                {
                    "type": "text",
                    "text": "README.md  markdown 512\nsrc/app.ts  typescript 128"
                }
            ]
        })
    );
}

#[test]
fn golden_codebase_retrieval_request_matches_core_contract() {
    let request: CoreRequest =
        serde_json::from_str(CODEBASE_RETRIEVAL_REQUEST).expect("request parses");

    assert_eq!(request.operation_name(), "codebase-retrieval");
    let value = serde_json::to_value(request).expect("request serializes");
    let golden: Value = serde_json::from_str(CODEBASE_RETRIEVAL_REQUEST).expect("golden parses");
    assert_eq!(value, golden);
}

#[test]
fn golden_multibyte_packed_segment_preserves_utf16_offsets() {
    let response: CoreResponse =
        serde_json::from_str(MULTIBYTE_PACKED_SEGMENT_RESPONSE).expect("response parses");
    let value = serde_json::to_value(response).expect("response serializes");
    let segment = &value["payload"]["pack"]["files"][0]["segments"][0];

    assert_eq!(segment["text"], "a🚀文");
    assert_eq!(segment["start_utf16"], 0);
    assert_eq!(segment["end_utf16"], 4);
    assert_eq!(
        segment["text"]
            .as_str()
            .expect("text string")
            .encode_utf16()
            .count(),
        4
    );
}

#[test]
fn golden_indexing_lifecycle_matches_current_convergence_contract() {
    let expected: LifecycleExpected =
        serde_json::from_str(INDEXING_LIFECYCLE_EXPECTED).expect("expected parses");
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path();
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::write(root.join("src/lib.rs"), "pub fn first() {}\n").expect("write lib");
    fs::write(root.join("README.md"), "# fixture\n").expect("write readme");
    let store = Store::open_in_memory().expect("store opens");

    let first = index_repository(&store, root).expect("first index");
    assert_run(
        &first,
        store.file_count().expect("file count"),
        &expected.first,
    );

    let second = index_repository(&store, root).expect("second index");
    assert_run(
        &second,
        store.file_count().expect("file count"),
        &expected.second,
    );

    fs::write(root.join("src/lib.rs"), "pub fn second() {}\n").expect("modify lib");
    fs::remove_file(root.join("README.md")).expect("delete readme");
    let third = index_repository(&store, root).expect("third index");
    assert_run(
        &third,
        store.file_count().expect("file count"),
        &expected.third,
    );

    assert_eq!(store.file_content("README.md").expect("read deleted"), None);
    assert_eq!(
        store
            .all_file_paths()
            .expect("paths read")
            .into_iter()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["src/lib.rs".to_string()])
    );
}

fn assert_run(
    actual: &randdb_core::IndexRunSummary,
    actual_file_count: u64,
    expected: &IndexRunExpected,
) {
    assert_eq!(actual.scanned_files, expected.scanned_files);
    assert_eq!(actual.added_files, expected.added_files);
    assert_eq!(actual.modified_files, expected.modified_files);
    assert_eq!(actual.deleted_files, expected.deleted_files);
    assert_eq!(actual.unchanged_files, expected.unchanged_files);
    assert_eq!(actual.skipped_embeddings, expected.skipped_embeddings);
    assert_eq!(actual_file_count, expected.file_count);
}
