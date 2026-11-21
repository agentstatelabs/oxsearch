//! Dangling indices against Elasticsearch, on any license: the list, empty
//! on the fixture, and an import and a delete of an index no node has,
//! which Elasticsearch refuses by its UUID. An index dangles when a node
//! rejoins with one the cluster has since forgotten, which a single node
//! can't do.

use oxsearch::Error;
use oxsearch::dangling_indices::{
    delete_dangling_index, import_dangling_index, list_dangling_indices,
};
use oxsearch_integration::client;

/// The UUID of an index no node has.
const MISSING: &str = "zmM4e0JtBkeUjiHD-MihPQ";

/// Checks that Elasticsearch found no dangling index with that UUID.
fn not_found<T: std::fmt::Debug>(result: Result<T, Error>) {
    match result {
        Err(Error::Elasticsearch(e)) => assert!(
            e.reason()
                .is_some_and(|r| r.contains("No dangling index found")),
            "{e}"
        ),
        other => panic!("an index no node has: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn dangling() {
    let client = client();
    let listed = client
        .send(&list_dangling_indices::Request::new())
        .await
        .unwrap();
    assert!(listed.dangling_indices.is_empty(), "{listed:?}");

    not_found(
        client
            .send(&import_dangling_index::Request::new(MISSING).accept_data_loss(true))
            .await,
    );
    not_found(
        client
            .send(&delete_dangling_index::Request::new(MISSING).accept_data_loss(true))
            .await,
    );
}
