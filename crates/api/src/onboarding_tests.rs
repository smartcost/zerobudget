use super::super::postgres_support::TestDatabase;
use super::*;
use obz_auth::{OnboardingService, Permission};
use obz_infrastructure::{
    Database, DatabaseConfig, PostgresIdentityRepository, PostgresOnboardingRepository,
};
use uuid::Uuid;
struct Setup {
    db: TestDatabase,
    owner: Database,
    runtime: Database,
    fixture: Fixture,
    cookie: String,
    csrf: String,
    user: Uuid,
}
impl Setup {
    async fn new() -> Self {
        let db = TestDatabase::create().await;
        let owner = Database::connect(DatabaseConfig::new(db.owner_url.clone(), true))
            .await
            .unwrap();
        owner.migrate().await.unwrap();
        PostgresIdentityRepository::from_database(&owner)
            .grant_runtime_role(&db.runtime_role)
            .await
            .unwrap();
        for value in [0, 1, 2, 3, 4] {
            match value {
                0 => obz_infrastructure::PostgresCycleRepository::new(owner.clone())
                    .grant_runtime_role(&db.runtime_role)
                    .await
                    .unwrap(),
                1 => obz_infrastructure::PostgresEnvelopeRepository::new(owner.clone())
                    .grant_runtime_role(&db.runtime_role)
                    .await
                    .unwrap(),
                2 => obz_infrastructure::PostgresExpenseRepository::new(owner.clone())
                    .grant_runtime_role(&db.runtime_role)
                    .await
                    .unwrap(),
                3 => obz_infrastructure::PostgresBankingRepository::new(owner.clone())
                    .grant_runtime_role(&db.runtime_role)
                    .await
                    .unwrap(),
                _ => obz_infrastructure::PostgresCreditRepository::new(owner.clone())
                    .grant_runtime_role(&db.runtime_role)
                    .await
                    .unwrap(),
            }
        }
        let runtime = Database::connect(DatabaseConfig::new(db.runtime_url.clone(), true))
            .await
            .unwrap();
        let mut fixture = Fixture::new(false, 300).await;
        fixture.state.users = Arc::new(PostgresIdentityRepository::from_database(&runtime));
        fixture.state.onboarding = Some(OnboardingService::new(Arc::new(
            PostgresOnboardingRepository::new(runtime.clone()),
        )));
        let (cookie, csrf) = fixture.authenticate().await;
        let response = fixture
            .request("GET", "/api/auth/me", Some(&cookie), None, None)
            .await;
        let data = json(response).await;
        let user = data["user_id"].as_str().unwrap().parse().unwrap();
        Self {
            db,
            owner,
            runtime,
            fixture,
            cookie,
            csrf,
            user,
        }
    }
    async fn get(&self) -> Response {
        self.fixture
            .request("GET", "/api/onboarding", Some(&self.cookie), None, None)
            .await
    }
    async fn post(&self) -> Response {
        router(self.fixture.state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/onboarding")
                    .header("cookie", &self.cookie)
                    .header("x-csrf-token", &self.csrf)
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap()
    }
    async fn audit(&self) -> Vec<String> {
        sqlx::query_scalar("SELECT action||':'||COALESCE(permission,'') FROM auth.audit_events WHERE user_id=$1 ORDER BY action,permission").bind(self.user).fetch_all(self.owner.pool()).await.unwrap()
    }
    async fn raw_complete(&self) -> Result<(), sqlx::Error> {
        let mut tx = self.runtime.pool().begin().await?;
        sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
            .bind(self.user.to_string())
            .execute(&mut *tx)
            .await?;
        let r = sqlx::query("SELECT auth.complete_budget_onboarding($1,$2,'alice')")
            .bind(self.user)
            .bind(&self.fixture.provider.issuer)
            .execute(&mut *tx)
            .await;
        match r {
            Ok(_) => tx.commit().await,
            Err(e) => {
                tx.rollback().await.unwrap();
                Err(e)
            }
        }
    }
}
async fn json(r: Response) -> Value {
    serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
fn code(e: sqlx::Error) -> String {
    e.as_database_error().unwrap().code().unwrap().into_owned()
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn onboarding_is_explicit_audited_idempotent_and_revocation_survives_retries() {
    let s = Setup::new().await;
    assert_eq!(json(s.get().await).await["status"], "pending");
    assert!(s.audit().await.is_empty());
    let (a, b) = tokio::join!(s.post(), s.post());
    assert_eq!(a.status(), StatusCode::OK);
    assert_eq!(json(a).await, json(b).await);
    assert_eq!(
        s.audit().await,
        vec!["grant:budget:read", "grant:budget:write", "onboard:"]
    );
    let admin = PostgresIdentityRepository::from_database(&s.owner);
    admin.revoke(s.user, Permission::ReadBudget).await.unwrap();
    let result = json(s.post().await).await;
    assert_eq!(result["status"], "completed");
    assert_eq!(result["next_step"], "contact_operator");
    assert!(result["setup"].is_null());
    assert_eq!(result["permissions"], json!(["budget:write"]));
    assert_eq!(s.audit().await.len(), 4);
    admin.set_enabled(s.user, false).await.unwrap();
    assert_eq!(s.post().await.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        s.raw_complete()
            .await
            .unwrap_err()
            .as_database_error()
            .unwrap()
            .code()
            .as_deref(),
        Some("42501")
    );
    admin.set_enabled(s.user, true).await.unwrap();
    assert_eq!(
        json(s.post().await).await["permissions"],
        json!(["budget:write"])
    );
    s.db.destroy().await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn onboarding_preserves_prior_operator_decisions_and_serializes_with_revocation() {
    let s = Setup::new().await;
    let mut tx = s.owner.pool().begin().await.unwrap();
    sqlx::query("SELECT id FROM auth.users WHERE id=$1 FOR UPDATE")
        .bind(s.user)
        .execute(&mut *tx)
        .await
        .unwrap();
    let mut request = Box::pin(s.post());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut request)
            .await
            .is_err()
    );
    sqlx::query("INSERT INTO auth.audit_events(id,user_id,action,permission) VALUES(uuidv7(),$1,'grant','budget:write'),(uuidv7(),$1,'revoke','budget:write')").bind(s.user).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(request.await.status(), StatusCode::CONFLICT);
    let status = json(s.get().await).await;
    assert_eq!(status["status"], "managed");
    assert_eq!(status["next_step"], "contact_operator");
    assert_eq!(status["permissions"], json!([]));
    s.db.destroy().await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn onboarding_function_scopes_privileges_and_rolls_back_grants_if_audit_fails() {
    let s = Setup::new().await;
    for statement in [
        "INSERT INTO auth.permissions VALUES(uuidv7(),'budget:write')",
        "INSERT INTO auth.budget_onboardings(user_id) VALUES(uuidv7())",
        "UPDATE auth.budget_onboardings SET completed_at=now()",
        "DELETE FROM auth.budget_onboardings",
        "UPDATE auth.users SET enabled=true",
    ] {
        assert_eq!(
            code(
                sqlx::query(statement)
                    .execute(s.runtime.pool())
                    .await
                    .unwrap_err()
            ),
            "42501"
        );
    }
    assert_eq!(
        code(
            sqlx::query("SELECT auth.complete_budget_onboarding($1,$2,'alice')")
                .bind(s.user)
                .bind(&s.fixture.provider.issuer)
                .execute(s.runtime.pool())
                .await
                .unwrap_err()
        ),
        "42501"
    );
    let mut tx = s.runtime.pool().begin().await.unwrap();
    sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
        .bind(s.user.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        code(
            sqlx::query(
                "SELECT auth.complete_budget_onboarding($1,'https://wrong.example','alice')"
            )
            .bind(s.user)
            .execute(&mut *tx)
            .await
            .unwrap_err()
        ),
        "42501"
    );
    tx.rollback().await.unwrap();
    sqlx::raw_sql("CREATE FUNCTION auth.fail_test_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'test audit failure'; END; $$; CREATE TRIGGER fail_test_audit BEFORE INSERT ON auth.audit_events FOR EACH ROW EXECUTE FUNCTION auth.fail_test_audit();").execute(s.owner.pool()).await.unwrap();
    assert_eq!(s.post().await.status(), StatusCode::SERVICE_UNAVAILABLE);
    let status = json(s.get().await).await;
    assert_eq!(status["status"], "pending");
    assert_eq!(status["permissions"], json!([]));
    assert!(s.audit().await.is_empty());
    sqlx::query("DROP TRIGGER fail_test_audit ON auth.audit_events")
        .execute(s.owner.pool())
        .await
        .unwrap();
    assert_eq!(s.post().await.status(), StatusCode::OK);
    s.db.destroy().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn administrative_revocation_waits_for_enrollment_and_wins_after_commit() {
    let s = Setup::new().await;
    let mut tx = s.runtime.pool().begin().await.unwrap();
    sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
        .bind(s.user.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT auth.complete_budget_onboarding($1,$2,'alice')")
        .bind(s.user)
        .bind(&s.fixture.provider.issuer)
        .execute(&mut *tx)
        .await
        .unwrap();
    let admin = PostgresIdentityRepository::from_database(&s.owner);
    let mut revoke = Box::pin(admin.revoke(s.user, Permission::WriteBudget));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut revoke)
            .await
            .is_err()
    );
    tx.commit().await.unwrap();
    revoke.await.unwrap();
    let status = json(s.post().await).await;
    assert_eq!(status["permissions"], json!(["budget:read"]));
    assert_eq!(status["next_step"], "contact_operator");
    assert_eq!(s.audit().await.len(), 4);
    s.db.destroy().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn uuid_v7_generation_defaults_and_insert_policy() {
    let s = Setup::new().await;
    assert_eq!(s.user.get_version_num(), 7);
    assert_eq!(s.post().await.status(), StatusCode::OK);
    let versions: Vec<i16> = sqlx::query_scalar(
        "SELECT uuid_extract_version(id) FROM auth.audit_events WHERE user_id=$1",
    )
    .bind(s.user)
    .fetch_all(s.owner.pool())
    .await
    .unwrap();
    assert_eq!(versions, vec![7, 7, 7]);
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO auth.audit_events(user_id,action) VALUES($1,'enable') RETURNING id",
    )
    .bind(s.user)
    .fetch_one(s.owner.pool())
    .await
    .unwrap();
    assert_eq!(id.get_version_num(), 7);
    for invalid in [
        "550e8400-e29b-41d4-a716-446655440000",
        "019a0000-0000-7000-0000-000000000001",
        "00000000-0000-0000-0000-000000000000",
    ] {
        let error =
            sqlx::query("INSERT INTO auth.audit_events(id,user_id,action) VALUES($1,$2,'enable')")
                .bind(invalid.parse::<Uuid>().unwrap())
                .bind(s.user)
                .execute(s.owner.pool())
                .await
                .unwrap_err();
        assert_eq!(code(error), "23514");
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_attribute a ON a.attrelid=c.oid AND a.attname='id' JOIN pg_attrdef d ON d.adrelid=c.oid AND d.adnum=a.attnum WHERE n.nspname IN ('auth','banking','budgeting') AND t.tgname='require_uuid_v7' AND pg_get_expr(d.adbin,d.adrelid)='uuidv7()'")
        .fetch_one(s.owner.pool()).await.unwrap();
    assert_eq!(count, 14);
    s.db.destroy().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn uuid_v7_migration_preserves_legacy_identity_and_references() {
    let db = TestDatabase::create().await;
    let owner = Database::connect(DatabaseConfig::new(db.owner_url.clone(), true))
        .await
        .unwrap();
    for migration in obz_infrastructure::MIGRATOR
        .iter()
        .filter(|m| m.version < 11)
    {
        sqlx::raw_sql(&migration.sql)
            .execute(owner.pool())
            .await
            .unwrap();
    }
    let legacy: Uuid = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
    sqlx::query("INSERT INTO auth.users(id,issuer,subject) VALUES($1,'https://legacy.invalid','opaque-subject')").bind(legacy).execute(owner.pool()).await.unwrap();
    sqlx::query(
        "INSERT INTO auth.audit_events(id,user_id,action) VALUES(gen_random_uuid(),$1,'enable')",
    )
    .bind(legacy)
    .execute(owner.pool())
    .await
    .unwrap();
    let migration = obz_infrastructure::MIGRATOR
        .iter()
        .find(|m| m.version == 11)
        .unwrap();
    sqlx::raw_sql(&migration.sql)
        .execute(owner.pool())
        .await
        .unwrap();
    let user: Uuid = sqlx::query_scalar("UPDATE auth.users SET last_login_at=now() WHERE issuer='https://legacy.invalid' AND subject='opaque-subject' RETURNING id").fetch_one(owner.pool()).await.unwrap();
    assert_eq!(user, legacy);
    let references: Vec<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM auth.audit_events WHERE user_id=$1")
            .bind(legacy)
            .fetch_all(owner.pool())
            .await
            .unwrap();
    assert_eq!(references, vec![legacy]);
    let new_id: Uuid = sqlx::query_scalar(
        "INSERT INTO auth.audit_events(user_id,action) VALUES($1,'disable') RETURNING id",
    )
    .bind(legacy)
    .fetch_one(owner.pool())
    .await
    .unwrap();
    assert_eq!(new_id.get_version_num(), 7);
    db.destroy().await;
}
