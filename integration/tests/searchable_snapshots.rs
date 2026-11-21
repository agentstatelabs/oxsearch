//! Searchable snapshots against Elasticsearch: an index snapshotted to a
//! shared file system repository (under the fixture's `path.repo`) and
//! mounted from it under another name, searched, its statistics read, and
//! its cache cleared; and the shared cache's statistics, on a node
//! without one. Mounting needs a license above Basic, so on Basic the test
//! checks that a mount is refused and reads the cache statistics.

use oxsearch::admin::indices::delete as delete_index;
use oxsearch::searchable_snapshots::{cache_stats, clear_cache, mount, stats};
use oxsearch::snapshot::{
    Repository, SharedFileSystemRepository, SharedFileSystemRepositorySettings, create,
    create_repository, delete, delete_repository,
};
use oxsearch::{Error, Refresh};
use oxsearch_docs::index;
use oxsearch_integration::{License, TestIndex, client};
use oxsearch_search::count;
use serde_json::json;

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn mounted() {
    let client = client();
    // The node has no shared cache: on a trial it reports one without
    // regions, and on Basic none at all.
    let cache = client.send(&cache_stats::Request::new()).await.unwrap();
    assert!(
        cache.nodes.values().all(|n| n
            .shared_cache
            .as_ref()
            .is_none_or(|c| c.num_regions == Some(0))),
        "{cache:?}"
    );
    if License::from_env() == License::Basic {
        let refused = client
            .send(
                &mount::Request::new("oxsearch-missing", "missing")
                    .body(mount::RequestBody::new("missing")),
            )
            .await;
        assert!(matches!(refused, Err(Error::License(_))), "{refused:?}");
        return;
    }

    // An index of three notes, snapshotted.
    let test = TestIndex::named("searchable");
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
    client
        .send(
            &create::Request::new(repository.as_str(), "notes")
                .wait_for_completion(true)
                .body(
                    create::RequestBody::new()
                        .indices(test.name())
                        .include_global_state(false),
                ),
        )
        .await
        .unwrap();

    // Mounted under another name, fully copied to the node, and searched.
    let mounted = format!("{}-mounted", test.name());
    let mount = client
        .send(
            &mount::Request::new(repository.as_str(), "notes")
                .wait_for_completion(true)
                .storage(mount::StorageOption::FullCopy)
                .body(mount::RequestBody::new(test.name()).renamed_index(mounted.as_str())),
        )
        .await
        .unwrap();
    let snapshot = mount.snapshot.expect("the mounted snapshot");
    assert_eq!(snapshot.snapshot.as_deref(), Some("notes"));
    let counted = client
        .send(&count::Request::new().index(mounted.as_str()))
        .await
        .unwrap();
    assert_eq!(counted.count, Some(3));

    let read = client
        .send(&stats::Request::new().index(mounted.as_str()))
        .await
        .unwrap();
    assert!(read.total.is_some(), "{read:?}");
    client
        .send(&clear_cache::Request::new().index(mounted.as_str()))
        .await
        .unwrap();

    client
        .send(&delete_index::Request::new(mounted.as_str()))
        .await
        .unwrap();
    client
        .send(&delete::Request::new(repository.as_str(), "notes"))
        .await
        .unwrap();
    client
        .send(&delete_repository::Request::new(repository.as_str()))
        .await
        .unwrap();
    test.delete(&client).await;
}
