//! Snapshots against Elasticsearch: a shared file system repository (under
//! the fixture's `path.repo`), and a snapshot of an index taken, read,
//! cloned, restored under a new name and deleted.

use oxsearch::Error;
use oxsearch::admin::indices::delete as delete_index;
use oxsearch::snapshot::{
    Repository, SharedFileSystemRepository, SharedFileSystemRepositorySettings, cleanup_repository,
    clone, create, create_repository, delete, delete_repository, get, get_repository,
    repository_analyze, restore, status, verify_repository,
};
use oxsearch::{Duration, Refresh};
use oxsearch_docs::index;
use oxsearch_integration::{TestIndex, client};
use oxsearch_search::count;
use serde_json::json;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn snapshots() {
    let client = client();
    let test = TestIndex::named("snapshotted");
    for n in 0..3 {
        client
            .send(
                &index::Request::new(test.name())
                    .id(n.to_string())
                    .refresh(Refresh::True)
                    .body(json!({"title": format!("note {n}")})),
            )
            .await
            .unwrap();
    }

    // A repository on the node's file system.
    let repository = format!("{}-repo", test.name());
    client
        .send(
            &create_repository::Request::new(repository.as_str()).body(Repository::from(
                SharedFileSystemRepository::new(SharedFileSystemRepositorySettings::new(
                    repository.clone(),
                )),
            )),
        )
        .await
        .unwrap();
    let verified = client
        .send(&verify_repository::Request::new(repository.as_str()))
        .await
        .unwrap();
    assert_eq!(verified.nodes.len(), 1);
    let repositories = client
        .send(&get_repository::Request::new().repository(repository.as_str()))
        .await
        .unwrap();
    assert!(
        matches!(
            repositories.get(&repository),
            Some(Repository::SharedFileSystem(_))
        ),
        "{repositories:?}"
    );

    // A snapshot of the index, waited for.
    let taken = client
        .send(
            &create::Request::new(repository.as_str(), "first")
                .wait_for_completion(true)
                .body(
                    create::RequestBody::new()
                        .indices(test.name())
                        .include_global_state(false),
                ),
        )
        .await
        .unwrap();
    let info = taken.snapshot.expect("the snapshot");
    assert_eq!(info.state.as_deref(), Some("SUCCESS"));
    assert_eq!(info.indices.as_deref(), Some(&[test.name().into()][..]));

    let read = client
        .send(&get::Request::new(repository.as_str(), "first"))
        .await
        .unwrap();
    assert_eq!(read.snapshots.as_ref().map(Vec::len), Some(1));
    let statuses = client
        .send(
            &status::Request::new()
                .repository(repository.as_str())
                .snapshot("first"),
        )
        .await
        .unwrap();
    assert_eq!(statuses.snapshots[0].state.as_deref(), Some("SUCCESS"));

    // A clone, which copies no data.
    client
        .send(
            &clone::Request::new(repository.as_str(), "first", "second")
                .body(clone::RequestBody::new(test.name())),
        )
        .await
        .unwrap();

    // Restored under a new name beside the original.
    let restored_name = format!("{}-restored", test.name());
    let restored = client
        .send(
            &restore::Request::new(repository.as_str(), "second")
                .wait_for_completion(true)
                .body(
                    restore::RequestBody::new()
                        .indices(test.name())
                        .rename_pattern("(.+)")
                        .rename_replacement("$1-restored"),
                ),
        )
        .await
        .unwrap();
    let indices = &restored.snapshot.as_ref().expect("the restore").indices;
    assert_eq!(indices, std::slice::from_ref(&restored_name));
    let counted = client
        .send(&count::Request::new().index(restored_name.as_str()))
        .await
        .unwrap();
    assert_eq!(counted.count, Some(3));
    client
        .send(&delete_index::Request::new(restored_name.as_str()))
        .await
        .unwrap();

    // An analysis of the repository's storage, kept small.
    let analysis = client
        .send(
            &repository_analyze::Request::new(repository.as_str())
                .blob_count(10)
                .max_blob_size("1mb")
                .timeout(Duration::String("60s".into())),
        )
        .await
        .unwrap();
    assert_eq!(analysis.blob_count, Some(10));

    for snapshot in ["first", "second"] {
        client
            .send(&delete::Request::new(repository.as_str(), snapshot))
            .await
            .unwrap();
    }
    let gone = client
        .send(&get::Request::new(repository.as_str(), "first"))
        .await;
    assert!(
        matches!(&gone, Err(Error::Elasticsearch(e)) if e.status == 404),
        "{gone:?}"
    );
    client
        .send(&cleanup_repository::Request::new(repository.as_str()))
        .await
        .unwrap();
    client
        .send(&delete_repository::Request::new(repository.as_str()))
        .await
        .unwrap();
    test.delete(&client).await;
}
