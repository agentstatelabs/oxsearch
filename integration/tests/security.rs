//! Security against Elasticsearch with security on: native users, roles,
//! role mappings and application privileges; API keys, cross-cluster API
//! keys and tokens; service accounts; user profiles; the security index's
//! settings and the nodes' statistics; and single sign-on, which the
//! fixture has no realms for, so its calls are read and refused.
//!
//! The fixture turns security off unless asked; these tests skip when it's
//! off. `oxsearch_integration`'s documentation says how to turn it on.

use oxsearch::Refresh;
use oxsearch::security::{
    activate_user_profile, authenticate, bulk_delete_role, bulk_put_role, bulk_update_api_keys,
    change_password, clear_api_key_cache, clear_cached_privileges, clear_cached_realms,
    clear_cached_roles, clear_cached_service_tokens, clone_api_key, create_api_key,
    create_cross_cluster_api_key, create_service_token, delegate_pki, delete_privileges,
    delete_role, delete_role_mapping, delete_service_token, delete_user, disable_user,
    disable_user_profile, enable_user, enable_user_profile, enroll_kibana, enroll_node,
    get_api_key, get_builtin_privileges, get_privileges, get_role, get_role_mapping,
    get_service_accounts, get_service_credentials, get_settings, get_stats, get_token, get_user,
    get_user_privileges, get_user_profile, grant_api_key, has_privileges,
    has_privileges_user_profile, invalidate_api_key, invalidate_token, oidc_authenticate,
    oidc_logout, oidc_prepare_authentication, put_privileges, put_role, put_role_mapping, put_user,
    query_api_keys, query_role, query_user, saml_authenticate, saml_complete_logout,
    saml_invalidate, saml_logout, saml_prepare_authentication, saml_service_provider_metadata,
    suggest_user_profiles, update_api_key, update_cross_cluster_api_key, update_settings,
    update_user_profile_data,
};
use oxsearch::shapes::IndexMap;
use oxsearch::{Error, Method};
use oxsearch_integration::{License, TestClient, TestIndex, client, secured, understood};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// A request body written as JSON, read into its generated type.
fn body<T: DeserializeOwned>(json: Value) -> T {
    serde_json::from_value(json).expect("the body fits its type")
}

/// The client, or `None` when the fixture runs without security.
fn secured_client() -> Option<TestClient> {
    if secured() {
        Some(client())
    } else {
        eprintln!("security is off in the fixture: skipped");
        None
    }
}

/// A name no other test uses, for a user, role, key or application.
fn name(prefix: &str) -> String {
    TestIndex::named(prefix).name().to_owned()
}

const PASSWORD: &str = "a-test-users-password";

/// A native user with the built-in `viewer` role.
async fn user(client: &TestClient, username: &str) {
    let created = client
        .send(
            &put_user::Request::new(username)
                .refresh(Refresh::WaitFor)
                .body(body::<put_user::RequestBody>(json!({
                    "password": PASSWORD,
                    "roles": ["viewer"],
                    "full_name": "Ada Tester",
                    "email": "ada@example.com",
                }))),
        )
        .await
        .unwrap();
    assert_eq!(created.created, Some(true));
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture with security on"]
async fn users_roles_and_privileges() {
    let Some(client) = secured_client() else {
        return;
    };
    let me = client.send(&authenticate::Request::new()).await.unwrap();
    assert_eq!(me.username.as_deref(), Some("elastic"));
    let builtin = client
        .send(&get_builtin_privileges::Request::new())
        .await
        .unwrap();
    assert!(!builtin.cluster.is_empty());
    let mine = client
        .send(&get_user_privileges::Request::new())
        .await
        .unwrap();
    assert!(mine.cluster.iter().any(|p| p == "all"), "{mine:?}");
    let has = client
        .send(
            &has_privileges::Request::new().body(body::<has_privileges::RequestBody>(
                json!({"cluster": ["monitor"]}),
            )),
        )
        .await
        .unwrap();
    assert_eq!(has.has_all_requested, Some(true));

    // Roles: one put, two in bulk, found by a query, and deleted.
    let role = name("role");
    let (second, third) = (format!("{role}-2"), format!("{role}-3"));
    let put = client
        .send(
            &put_role::Request::new(role.as_str())
                .refresh(Refresh::WaitFor)
                .body(body::<put_role::RequestBody>(json!({
                    "cluster": ["monitor"],
                    "indices": [{"names": ["logs-*"], "privileges": ["read"]}],
                    "description": "reads logs",
                }))),
        )
        .await
        .unwrap();
    assert_eq!(put.role.and_then(|r| r.created), Some(true));
    let read = client
        .send(&get_role::Request::new().name(role.as_str()))
        .await
        .unwrap();
    assert!(read.contains_key(role.as_str()), "{read:?}");
    let bulk = client
        .send(
            &bulk_put_role::Request::new()
                .refresh(Refresh::WaitFor)
                .body(body::<bulk_put_role::RequestBody>(json!({"roles": {
                    second.clone(): {"cluster": ["monitor"]},
                    third.clone(): {"cluster": ["monitor"]},
                }}))),
        )
        .await
        .unwrap();
    assert_eq!(bulk.created.map(|c| c.len()), Some(2));
    let found = client
        .send(
            &query_role::Request::new().body(body::<query_role::RequestBody>(json!({
                "query": {"prefix": {"name": role}},
            }))),
        )
        .await
        .unwrap();
    assert_eq!(found.count, Some(3), "{found:?}");
    client
        .send(&clear_cached_roles::Request::new(role.as_str()))
        .await
        .unwrap();
    let deleted = client
        .send(
            &bulk_delete_role::Request::new()
                .refresh(Refresh::WaitFor)
                .body(bulk_delete_role::RequestBody::new(vec![
                    second.clone(),
                    third.clone(),
                ])),
        )
        .await
        .unwrap();
    assert_eq!(deleted.deleted.map(|d| d.len()), Some(2));

    // A native user: found, its password changed, disabled and enabled.
    let username = name("user");
    user(&client, &username).await;
    let users = client
        .send(&get_user::Request::new().username(username.as_str()))
        .await
        .unwrap();
    assert!(users.contains_key(username.as_str()), "{users:?}");
    let queried = client
        .send(
            &query_user::Request::new().body(body::<query_user::RequestBody>(json!({
                "query": {"term": {"username": username}},
            }))),
        )
        .await
        .unwrap();
    assert_eq!(queried.count, Some(1), "{queried:?}");
    client
        .send(
            &change_password::Request::new()
                .username(username.as_str())
                .body(body::<change_password::RequestBody>(
                    json!({"password": "another-test-password"}),
                )),
        )
        .await
        .unwrap();
    client
        .send(&disable_user::Request::new(username.as_str()))
        .await
        .unwrap();
    client
        .send(&enable_user::Request::new(username.as_str()))
        .await
        .unwrap();
    client
        .send(&clear_cached_realms::Request::new("*").usernames(vec![username.clone()]))
        .await
        .unwrap();

    // A role mapping for users of any realm with that name.
    let mapping = name("mapping");
    let mapped = client
        .send(
            &put_role_mapping::Request::new(mapping.as_str()).body(body::<
                put_role_mapping::RequestBody,
            >(json!({
                "roles": [role],
                "enabled": true,
                "rules": {"all": [
                    {"field": {"username": username}},
                    {"except": {"field": {"realm.name": "nowhere"}}},
                ]},
            }))),
        )
        .await
        .unwrap();
    assert_eq!(mapped.role_mapping.and_then(|m| m.created), Some(true));
    let mappings = client
        .send(&get_role_mapping::Request::new().name(mapping.as_str()))
        .await
        .unwrap();
    assert!(mappings.contains_key(mapping.as_str()), "{mappings:?}");
    let gone = client
        .send(&delete_role_mapping::Request::new(mapping.as_str()))
        .await
        .unwrap();
    assert_eq!(gone.found, Some(true));

    // Application privileges.
    let application = name("app").replace('-', "_");
    client
        .send(&put_privileges::Request::new().body(body::<
            IndexMap<String, IndexMap<String, put_privileges::Actions>>,
        >(json!({
            application.clone(): {"read": {"actions": ["data:read/*"]}},
        }))))
        .await
        .unwrap();
    let privileges = client
        .send(&get_privileges::Request::new().application(application.as_str()))
        .await
        .unwrap();
    assert!(
        privileges.contains_key(application.as_str()),
        "{privileges:?}"
    );
    client
        .send(&clear_cached_privileges::Request::new(application.as_str()))
        .await
        .unwrap();
    client
        .send(&delete_privileges::Request::new(
            application.as_str(),
            "read",
        ))
        .await
        .unwrap();

    let gone = client
        .send(&delete_user::Request::new(username.as_str()))
        .await
        .unwrap();
    assert_eq!(gone.found, Some(true));
    let gone = client
        .send(&delete_role::Request::new(role.as_str()))
        .await
        .unwrap();
    assert_eq!(gone.found, Some(true));
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture with security on"]
async fn api_keys_and_tokens() {
    let Some(client) = secured_client() else {
        return;
    };
    let key_name = name("key");
    let created = client
        .send(
            &create_api_key::Request::new().body(body::<create_api_key::RequestBody>(json!({
                "name": key_name,
                "expiration": "1d",
                "metadata": {"purpose": "tests"},
            }))),
        )
        .await
        .unwrap();
    let id = created.id.expect("the key's id");
    assert!(created.encoded.is_some());
    let read = client
        .send(&get_api_key::Request::new().id(id.as_str()))
        .await
        .unwrap();
    assert_eq!(read.api_keys.len(), 1);
    let found = client
        .send(
            &query_api_keys::Request::new().body(body::<query_api_keys::RequestBody>(json!({
                "query": {"term": {"name": key_name}},
            }))),
        )
        .await
        .unwrap();
    assert_eq!(found.count, Some(1), "{found:?}");
    let updated =
        client
            .send(&update_api_key::Request::new(id.as_str()).body(body::<
                update_api_key::RequestBody,
            >(json!({
                "metadata": {"purpose": "tests", "updated": true},
            }))))
            .await
            .unwrap();
    assert_eq!(updated.updated, Some(true));
    let bulk = client
        .send(&bulk_update_api_keys::Request::new().body(
            body::<bulk_update_api_keys::RequestBody>(json!({
                "ids": [id],
                "metadata": {"purpose": "bulk"},
            })),
        ))
        .await
        .unwrap();
    assert_eq!(bulk.updated, std::slice::from_ref(&id));
    client
        .send(&clear_api_key_cache::Request::new(id.as_str()))
        .await
        .unwrap();
    // Cloning a key is experimental: whatever the server says, it read it.
    match client
        .send(
            &clone_api_key::Request::new().body(clone_api_key::RequestBody::new(
                created.encoded.clone().unwrap_or_default(),
            )),
        )
        .await
    {
        Ok(_) => {}
        Err(e) => assert!(understood(&e), "{e}"),
    }

    // A key granted on behalf of another user, by their password.
    let username = name("granted");
    user(&client, &username).await;
    let granted = client
        .send(
            &grant_api_key::Request::new().body(body::<grant_api_key::RequestBody>(json!({
                "grant_type": "password",
                "username": username,
                "password": PASSWORD,
                "api_key": {"name": format!("{key_name}-granted")},
            }))),
        )
        .await
        .unwrap();
    let granted_id = granted.id.expect("the granted key's id");
    let invalidated =
        client
            .send(&invalidate_api_key::Request::new().body(
                body::<invalidate_api_key::RequestBody>(json!({
                    "ids": [id, granted_id],
                })),
            ))
            .await
            .unwrap();
    assert_eq!(invalidated.invalidated_api_keys.len(), 2);

    // A key for another cluster's searches, where the license allows it.
    match client
        .send(&create_cross_cluster_api_key::Request::new().body(body::<
            create_cross_cluster_api_key::RequestBody,
        >(json!({
            "name": format!("{key_name}-remote"),
            "access": {"search": [{"names": ["logs-*"]}]},
        }))))
        .await
    {
        Ok(remote) => {
            let remote_id = remote.id.expect("the key's id");
            let updated = client
                .send(
                    &update_cross_cluster_api_key::Request::new(remote_id.as_str()).body(body::<
                        update_cross_cluster_api_key::RequestBody,
                    >(
                        json!({"access": {"search": [{"names": ["metrics-*"]}]}}),
                    )),
                )
                .await
                .unwrap();
            assert_eq!(updated.updated, Some(true));
            client
                .send(&invalidate_api_key::Request::new().body(body::<
                    invalidate_api_key::RequestBody,
                >(
                    json!({"ids": [remote_id]})
                )))
                .await
                .unwrap();
        }
        Err(e) => assert!(understood(&e), "{e}"),
    }

    // Tokens: one for the user's password, refreshed, then invalidated.
    // The token service needs a license above Basic.
    let token = client
        .send(
            &get_token::Request::new().body(body::<get_token::RequestBody>(json!({
                "grant_type": "password",
                "username": username,
                "password": PASSWORD,
            }))),
        )
        .await;
    if License::from_env() == License::Basic {
        assert!(matches!(token, Err(Error::License(_))), "{token:?}");
        client
            .send(&delete_user::Request::new(username.as_str()))
            .await
            .unwrap();
        return;
    }
    let token = token.unwrap();
    let refreshed = client
        .send(
            &get_token::Request::new().body(body::<get_token::RequestBody>(json!({
                "grant_type": "refresh_token",
                "refresh_token": token.refresh_token.expect("a refresh token"),
            }))),
        )
        .await
        .unwrap();
    let invalidated = client
        .send(
            &invalidate_token::Request::new().body(body::<invalidate_token::RequestBody>(json!({
                "token": refreshed.access_token.expect("an access token"),
            }))),
        )
        .await
        .unwrap();
    assert_eq!(invalidated.invalidated_tokens, Some(1));
    client
        .send(&delete_user::Request::new(username.as_str()))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture with security on"]
async fn service_accounts() {
    let Some(client) = secured_client() else {
        return;
    };
    let accounts = client
        .send(&get_service_accounts::Request::new())
        .await
        .unwrap();
    assert!(
        accounts.contains_key("elastic/fleet-server"),
        "{accounts:?}"
    );
    let token = name("token");
    let created = client
        .send(&create_service_token::Request::new("elastic", "fleet-server").name(token.as_str()))
        .await
        .unwrap();
    assert!(created.token.and_then(|t| t.value).is_some());
    let credentials = client
        .send(&get_service_credentials::Request::new(
            "elastic",
            "fleet-server",
        ))
        .await
        .unwrap();
    assert!(
        credentials.tokens.contains_key(token.as_str()),
        "{credentials:?}"
    );
    client
        .send(&clear_cached_service_tokens::Request::new(
            "elastic",
            "fleet-server",
            token.as_str(),
        ))
        .await
        .unwrap();
    let gone = client
        .send(&delete_service_token::Request::new(
            "elastic",
            "fleet-server",
            token.as_str(),
        ))
        .await
        .unwrap();
    assert_eq!(gone.found, Some(true));
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture with security on"]
async fn user_profiles() {
    let Some(client) = secured_client() else {
        return;
    };
    let username = name("profiled");
    user(&client, &username).await;
    let profile = client
        .send(&activate_user_profile::Request::new().body(
            body::<activate_user_profile::RequestBody>(json!({
                "grant_type": "password",
                "username": username,
                "password": PASSWORD,
            })),
        ))
        .await
        .unwrap();
    let uid = profile.uid.clone().expect("the profile's uid");
    client
        .send(
            &update_user_profile_data::Request::new(uid.as_str())
                .refresh(Refresh::WaitFor)
                .body(body::<update_user_profile_data::RequestBody>(json!({
                    "labels": {"direction": "north"},
                    "data": {"app1": {"theme": "dark"}},
                }))),
        )
        .await
        .unwrap();
    let read = client
        .send(&get_user_profile::Request::new(uid.clone()).data("app1"))
        .await
        .unwrap();
    assert_eq!(read.profiles[0].data["app1"]["theme"], "dark");
    let suggested = client
        .send(&suggest_user_profiles::Request::new().body(
            body::<suggest_user_profiles::RequestBody>(json!({
                "name": "Ada Tester",
            })),
        ))
        .await
        .unwrap();
    assert!(!suggested.profiles.is_empty(), "{suggested:?}");
    let has = client
        .send(&has_privileges_user_profile::Request::new().body(body::<
            has_privileges_user_profile::RequestBody,
        >(json!({
            "uids": [uid],
            "privileges": {"index": [{"names": ["logs"], "privileges": ["read"]}]},
        }))))
        .await
        .unwrap();
    // The viewer role reads every index but the system's.
    assert_eq!(has.has_privilege_uids, std::slice::from_ref(&uid));
    client
        .send(&disable_user_profile::Request::new(uid.as_str()))
        .await
        .unwrap();
    client
        .send(&enable_user_profile::Request::new(uid.as_str()))
        .await
        .unwrap();
    client
        .send(&delete_user::Request::new(username.as_str()))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture with security on"]
async fn settings_and_statistics() {
    let Some(client) = secured_client() else {
        return;
    };
    client.send(&get_settings::Request::new()).await.unwrap();
    // The specification, and its own example, write the index settings
    // nested under `index`; Elasticsearch 9.5 refuses that and takes them
    // only flat, as `index.auto_expand_replicas`, which the type can't
    // write. So the typed request is read and refused, and the raw call
    // sets them.
    let typed = client
        .send(
            &update_settings::Request::new().body(body::<update_settings::RequestBody>(json!({
                "security": {"index": {"auto_expand_replicas": "0-1"}},
            }))),
        )
        .await;
    match typed {
        Ok(updated) => assert_eq!(updated.acknowledged, Some(true)),
        Err(e) => assert!(understood(&e), "{e}"),
    }
    let raw = client
        .send_raw(
            Method::Put,
            "/_security/settings",
            Some(&json!({"security": {"index.auto_expand_replicas": "0-1"}})),
        )
        .await
        .unwrap();
    assert_eq!(raw["acknowledged"], true);
    let stats = client.send(&get_stats::Request::new()).await.unwrap();
    assert!(!stats.nodes.is_empty(), "{stats:?}");
}

/// A self-signed certificate made for these tests, whose key was thrown
/// away: `delegate_pki` reads it, and finds no PKI realm to trust it.
const CERTIFICATE: &str = concat!(
    "MIIBZTCCAQygAwIBAgIUd5jK6oqCrHw7Ze7EPAiwSAmh/q8wCgYIKoZIzj0EAwIw",
    "GDEWMBQGA1UEAwwNb3hzZWFyY2gtdGVzdDAgFw0yNjExMjAyMTM4MzFaGA8yMTI2",
    "MTAyNzIxMzgzMVowGDEWMBQGA1UEAwwNb3hzZWFyY2gtdGVzdDBZMBMGByqGSM49",
    "AgEGCCqGSM49AwEHA0IABEWWfhGlwIQvBCq9VgYRzE/sYvqhKAZiodpR3JIlvDDp",
    "kBZxqpnH61LFIzrbMZpVYEUnDgcDyjyTcQAYZHQGXxujMjAwMB0GA1UdDgQWBBSu",
    "RgGw9//rzLju/lpVyg91kFFd0jAPBgNVHRMBAf8EBTADAQH/MAoGCCqGSM49BAMC",
    "A0cAMEQCIGP3k7nWSfo0b6JWNf+tuw6YuW50XpJ1JpgW0H3tbAxUAiBE7WHjgvti",
    "eSfq+KVrdZk6hjStayx0yfEeBi+mKHUhJQ==",
);

/// Elasticsearch read the request, and refused it because the fixture has
/// no such realm, enrollment or certificate.
fn refused<T: std::fmt::Debug>(result: Result<T, Error>) {
    match result {
        Err(e) => assert!(understood(&e), "{e}"),
        Ok(answer) => panic!("expected a refusal: {answer:?}"),
    }
}

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture with security on"]
async fn single_sign_on_without_realms() {
    let Some(client) = secured_client() else {
        return;
    };
    refused(
        client
            .send(&saml_prepare_authentication::Request::new().body(body::<
                saml_prepare_authentication::RequestBody,
            >(json!({
                "realm": "saml1",
            }))))
            .await,
    );
    refused(
        client
            .send(
                &saml_authenticate::Request::new().body(body::<saml_authenticate::RequestBody>(
                    json!({
                        "content": "PHNhbWxwOlJlc3BvbnNlLz4=",
                        "ids": ["_1c368075e0b3"],
                    }),
                )),
            )
            .await,
    );
    refused(
        client
            .send(
                &saml_logout::Request::new().body(body::<saml_logout::RequestBody>(
                    json!({"token": "not-a-token"}),
                )),
            )
            .await,
    );
    refused(
        client
            .send(
                &saml_invalidate::Request::new().body(body::<saml_invalidate::RequestBody>(
                    json!({
                        "query_string": "SAMLRequest=nZFda4MwFIb%2FiuS%2BmviRpqFaClKQdbvo2ri",
                        "realm": "saml1",
                    }),
                )),
            )
            .await,
    );
    refused(
        client
            .send(&saml_complete_logout::Request::new().body(body::<
                saml_complete_logout::RequestBody,
            >(json!({
                "realm": "saml1",
                "ids": ["_1c368075e0b3"],
                "query_string": "SAMLResponse=fZHLasMwEEVbr1sR",
            }))))
            .await,
    );
    refused(
        client
            .send(&saml_service_provider_metadata::Request::new("saml1"))
            .await,
    );
    refused(
        client
            .send(&oidc_prepare_authentication::Request::new().body(body::<
                oidc_prepare_authentication::RequestBody,
            >(json!({
                "realm": "oidc1",
            }))))
            .await,
    );
    refused(
        client
            .send(&oidc_authenticate::Request::new().body(body::<oidc_authenticate::RequestBody>(json!({
                "realm": "oidc1",
                "redirect_uri": "https://kibana.example.org/api/security/oidc/callback?code=x&state=y",
                "state": "y",
                "nonce": "n",
            }))))
            .await,
    );
    refused(
        client
            .send(
                &oidc_logout::Request::new().body(body::<oidc_logout::RequestBody>(
                    json!({"token": "not-a-token"}),
                )),
            )
            .await,
    );
    refused(
        client
            .send(
                &delegate_pki::Request::new().body(body::<delegate_pki::RequestBody>(json!({
                    "x509_certificate_chain": [CERTIFICATE],
                }))),
            )
            .await,
    );
    refused(client.send(&enroll_kibana::Request::new()).await);
    refused(client.send(&enroll_node::Request::new()).await);
    // The raw call works with security on, as the elastic user.
    let health = client
        .send_raw(Method::Get, "/_cluster/health", None)
        .await
        .unwrap();
    assert!(health["status"].is_string());
}
