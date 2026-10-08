//! Integration tests for the PostgreSQL backend and the legacy TS -> canonical
//! data migration, run against a real Postgres via testcontainers.
//!
//! These require Docker. When Docker is unavailable (e.g. local dev without a
//! daemon) each test logs a skip notice and returns green instead of failing.

use std::sync::Arc;

use alloy::primitives::Address;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::ContainerAsync;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

use odis_signer::account_service::MockAccountService;
use odis_signer::config::{Config, DatabaseConfig, KeystoreType};
use odis_signer::key_management::{KeyProvider, MockKeyProvider};
use odis_signer::request_service::{PnpRequestService, PostgresPnpRequestService};
use odis_signer::server::build_router_with_services;

/// The same checksummed address in mixed and lower case; both must collapse to
/// one canonical account during migration.
const VITALIK_CHECKSUMMED: &str = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045";
const VITALIK_LOWERCASE: &str = "0xd8da6bf26964af9d7eed9e03e53415d37aa96045";
const TEST_ADDRESS: &str = "0x0000000000000000000000000000000000007E57";

/// Start a Postgres container, returning `None` (with a skip notice) when Docker
/// is unavailable. The returned container must stay in scope for the test.
///
/// When `ODIS_REQUIRE_POSTGRES_TESTS` is set (CI does this), a Docker failure is
/// a hard error instead of a skip, so the Postgres suite can never silently
/// no-op and hide a regression.
async fn start_postgres() -> Option<(ContainerAsync<Postgres>, String)> {
    match Postgres::default().start().await {
        Ok(node) => {
            let port = node
                .get_host_port_ipv4(5432_u16)
                .await
                .expect("failed to get mapped Postgres port");
            let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
            Some((node, url))
        }
        Err(e) => {
            if std::env::var_os("ODIS_REQUIRE_POSTGRES_TESTS").is_some() {
                panic!("Postgres tests are required but Docker is unavailable: {e}");
            }
            eprintln!("skipping Postgres integration test (Docker unavailable): {e}");
            None
        }
    }
}

/// Create the legacy TS tables and seed them with data exercising every cleanup
/// path: mixed casing, NULL num_lookups, NULL signature, unparseable addresses,
/// and duplicate (address, blinded_query) pairs.
async fn seed_legacy(pool: &PgPool) {
    sqlx::query(
        r#"CREATE TABLE "accountsOnChain" (
            address varchar(255) NOT NULL PRIMARY KEY,
            created_at timestamptz NOT NULL,
            num_lookups integer
        )"#,
    )
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        r#"CREATE TABLE "requestsOnChain" (
            caller_address varchar(255) NOT NULL,
            timestamp timestamptz NOT NULL,
            blinded_query varchar(255) NOT NULL,
            signature varchar(255),
            PRIMARY KEY (caller_address, blinded_query, timestamp)
        )"#,
    )
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        r#"INSERT INTO "accountsOnChain" (address, created_at, num_lookups) VALUES
            ($1, '2020-01-01 00:00:00+00', 3),
            ($2, '2021-01-01 00:00:00+00', 4),
            ($3, '2022-01-01 00:00:00+00', NULL),
            ('not-an-address', '2022-06-01 00:00:00+00', 9)"#,
    )
    .bind(VITALIK_CHECKSUMMED)
    .bind(VITALIK_LOWERCASE)
    .bind(TEST_ADDRESS)
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        r#"INSERT INTO "requestsOnChain" (caller_address, timestamp, blinded_query, signature) VALUES
            ($1, '2023-01-01 00:00:00+00', 'q1', 'sigA'),
            ($2, '2023-06-01 00:00:00+00', 'q1', 'sigB'),
            ($3, '2023-01-01 00:00:00+00', 'q2', NULL),
            ('bad-addr',                   '2023-01-01 00:00:00+00', 'q3', 'sigC')"#,
    )
    .bind(VITALIK_LOWERCASE)
    .bind(VITALIK_CHECKSUMMED)
    .bind(TEST_ADDRESS)
    .execute(pool)
    .await
    .unwrap();
}

fn pg_config(url: &str, migrate_legacy_data: bool) -> Config {
    Config {
        server_port: 8080,
        pnp_api_enabled: true,
        keystore_type: KeystoreType::Mock,
        pnp_key_name_base: "phoneNumberPrivacy".to_string(),
        pnp_latest_key_version: 1,
        database: DatabaseConfig::Postgres {
            url: url.to_string(),
        },
        migrate_legacy_data,
        blockchain_provider: None,
        chain_id: 44787,
        accounts_contract_address: None,
        odis_payments_contract_address: None,
        full_node_retry_count: 5,
        full_node_retry_delay_ms: 100,
        timeout_ms: 5000,
        query_price_per_cusd: 0.001,
        google_project_id: None,
        request_pruning_days: 7,
        request_pruning_interval_secs: 86400,
    }
}

#[tokio::test]
async fn postgres_backend_behaviour() {
    let Some((_node, url)) = start_postgres().await else {
        return;
    };
    let svc = PostgresPnpRequestService::new(&url).await.unwrap();

    let addr: Address = TEST_ADDRESS.parse().unwrap();
    let other: Address = "0x0000000000000000000000000000000000000001"
        .parse()
        .unwrap();

    assert_eq!(svc.get_used_quota(addr).await.unwrap(), 0);
    assert_eq!(svc.get_duplicate_request(addr, "q1").await.unwrap(), None);

    svc.record_request(addr, "q1", "sig1").await.unwrap();
    assert_eq!(svc.get_used_quota(addr).await.unwrap(), 1);
    assert_eq!(
        svc.get_duplicate_request(addr, "q1").await.unwrap(),
        Some("sig1".to_string())
    );

    // A different query increments quota again.
    svc.record_request(addr, "q2", "sig2").await.unwrap();
    assert_eq!(svc.get_used_quota(addr).await.unwrap(), 2);

    // Accounts are independent.
    assert_eq!(svc.get_used_quota(other).await.unwrap(), 0);

    // Freshly inserted rows aren't pruned by a 7-day cutoff, but a 0-day cutoff
    // removes everything.
    assert_eq!(svc.delete_old_requests(7).await.unwrap(), 0);
    assert_eq!(svc.delete_old_requests(0).await.unwrap(), 2);
    assert_eq!(svc.get_duplicate_request(addr, "q1").await.unwrap(), None);
}

#[tokio::test]
async fn legacy_migration_normalizes_and_dedups() {
    let Some((_node, url)) = start_postgres().await else {
        return;
    };

    let setup_pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    seed_legacy(&setup_pool).await;

    let svc = PostgresPnpRequestService::new(&url).await.unwrap();
    let report = svc.run_legacy_migration().await.unwrap();

    assert!(report.legacy_tables_present);
    assert!(!report.already_migrated);
    // Two Vitalik rows (mixed + lower case) plus the NULL-lookup account; the
    // unparseable address is skipped.
    assert_eq!(report.accounts_imported, 3);
    assert_eq!(report.accounts_skipped_unparseable, 1);
    // Two q1 rows import (deduped to one canonical row); NULL signature and
    // unparseable address are skipped.
    assert_eq!(report.requests_imported, 2);
    assert_eq!(report.requests_skipped_null_signature, 1);
    assert_eq!(report.requests_skipped_unparseable, 1);

    let vitalik: Address = VITALIK_CHECKSUMMED.parse().unwrap();
    let test_addr: Address = TEST_ADDRESS.parse().unwrap();

    // Mixed + lower casings collapse to one checksummed account with summed quota.
    assert_eq!(svc.get_used_quota(vitalik).await.unwrap(), 7);
    // NULL num_lookups coalesces to 0.
    assert_eq!(svc.get_used_quota(test_addr).await.unwrap(), 0);
    // Duplicate (address, query) keeps the most recent signature.
    assert_eq!(
        svc.get_duplicate_request(vitalik, "q1").await.unwrap(),
        Some("sigB".to_string())
    );
    // The NULL-signature request was dropped.
    assert_eq!(
        svc.get_duplicate_request(test_addr, "q2").await.unwrap(),
        None
    );

    // Idempotent: a second run is a no-op thanks to the marker.
    let report2 = svc.run_legacy_migration().await.unwrap();
    assert!(report2.already_migrated);
    assert_eq!(report2.accounts_imported, 0);
    assert_eq!(report2.requests_imported, 0);
}

#[tokio::test]
async fn legacy_migration_no_op_without_legacy_tables() {
    let Some((_node, url)) = start_postgres().await else {
        return;
    };
    let svc = PostgresPnpRequestService::new(&url).await.unwrap();

    let report = svc.run_legacy_migration().await.unwrap();
    assert!(!report.legacy_tables_present);
    assert!(!report.already_migrated);
    assert_eq!(report.accounts_imported, 0);
    assert_eq!(report.requests_imported, 0);
}

#[tokio::test]
async fn full_stack_serves_migrated_quota() {
    let Some((_node, url)) = start_postgres().await else {
        return;
    };

    let setup_pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    seed_legacy(&setup_pool).await;

    // Building the router with migrate_legacy_data runs the ETL on startup.
    let account_service = Arc::new(MockAccountService::new(None, 100));
    let key_provider: Arc<dyn KeyProvider> = Arc::new(MockKeyProvider::new());
    let app = build_router_with_services(pg_config(&url, true), account_service, key_provider)
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/quotaStatus")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"account":"{VITALIK_CHECKSUMMED}"}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    // Quota was migrated from the legacy tables and is served from Postgres.
    assert_eq!(json["performedQueryCount"], 7);
    assert_eq!(json["totalQuota"], 100);
}

#[tokio::test]
async fn legacy_migration_paginates_across_pages() {
    let Some((_node, url)) = start_postgres().await else {
        return;
    };

    let setup_pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();

    sqlx::query(
        r#"CREATE TABLE "accountsOnChain" (
            address varchar(255) NOT NULL PRIMARY KEY,
            created_at timestamptz NOT NULL,
            num_lookups integer
        )"#,
    )
    .execute(&setup_pool)
    .await
    .unwrap();
    sqlx::query(
        r#"CREATE TABLE "requestsOnChain" (
            caller_address varchar(255) NOT NULL,
            timestamp timestamptz NOT NULL,
            blinded_query varchar(255) NOT NULL,
            signature varchar(255),
            PRIMARY KEY (caller_address, blinded_query, timestamp)
        )"#,
    )
    .execute(&setup_pool)
    .await
    .unwrap();

    // Seed more rows than PAGE_SIZE (1000) so keyset pagination spans several
    // pages. Distinct zero-padded hex addresses keep string and numeric order
    // aligned, and are valid (lowercase) addresses that re-checksum cleanly.
    let rows: i64 = 2500;
    sqlx::query(
        r#"INSERT INTO "accountsOnChain" (address, created_at, num_lookups)
           SELECT '0x' || lpad(to_hex(g), 40, '0'), now(), 1
           FROM generate_series(1, $1) AS g"#,
    )
    .bind(rows)
    .execute(&setup_pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO "requestsOnChain" (caller_address, timestamp, blinded_query, signature)
           SELECT '0x' || lpad(to_hex(g), 40, '0'), now(), 'q', 'sig'
           FROM generate_series(1, $1) AS g"#,
    )
    .bind(rows)
    .execute(&setup_pool)
    .await
    .unwrap();

    let svc = PostgresPnpRequestService::new(&url).await.unwrap();
    let report = svc.run_legacy_migration().await.unwrap();

    assert_eq!(report.accounts_imported, rows as u64);
    assert_eq!(report.requests_imported, rows as u64);
    assert_eq!(report.accounts_skipped_unparseable, 0);
    assert_eq!(report.requests_skipped_unparseable, 0);

    // Spot-check a row from the last page (g = 2500 -> 0x...09c4).
    let last: Address = format!("0x{:040x}", 2500u64).parse().unwrap();
    assert_eq!(svc.get_used_quota(last).await.unwrap(), 1);
    assert_eq!(
        svc.get_duplicate_request(last, "q").await.unwrap(),
        Some("sig".to_string())
    );
}
