//! The license API: the cluster's license, and whether it may start a trial
//! or go back to Basic. Starting either would change the license under the
//! other tests, so they aren't sent.

use oxsearch::license::{LicenseStatus, LicenseType, get, get_basic_status, get_trial_status};
use oxsearch_integration::{License, client};

#[tokio::test]
#[ignore = "needs the Elasticsearch fixture: scripts/es-fixture.sh up"]
async fn reads_the_license() {
    let client = client();
    let license = client
        .send(&get::Request::new())
        .await
        .unwrap()
        .license
        .expect("a license");
    let expected = match License::from_env() {
        License::Basic => LicenseType::Basic,
        License::Trial => LicenseType::Trial,
    };
    assert_eq!(license.r#type, Some(expected));
    assert_eq!(license.status, Some(LicenseStatus::Active));

    let trial = client
        .send(&get_trial_status::Request::new())
        .await
        .unwrap();
    let basic = client
        .send(&get_basic_status::Request::new())
        .await
        .unwrap();
    match License::from_env() {
        // A fresh cluster may start its one trial; it's on Basic already.
        License::Basic => {
            assert_eq!(trial.eligible_to_start_trial, Some(true));
            assert_eq!(basic.eligible_to_start_basic, Some(false));
        }
        // A trial was started, so not another; Basic is always there.
        License::Trial => {
            assert_eq!(trial.eligible_to_start_trial, Some(false));
            assert_eq!(basic.eligible_to_start_basic, Some(true));
        }
    }
}
