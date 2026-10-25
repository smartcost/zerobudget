use super::super::postgres_support::TestDatabase;
use super::*;
use obz_auth::Permission::{ReadBudget, WriteBudget};
use obz_cycle::{CycleError, EnvelopeService};
use obz_infrastructure::{
    Database, DatabaseConfig, PostgresCycleRepository, PostgresEnvelopeRepository,
    PostgresIdentityRepository,
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
    cycle: Uuid,
}
async fn json(response: Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
async fn request(
    f: &Fixture,
    cookie: &str,
    csrf: Option<&str>,
    method: &str,
    path: &str,
    body: Value,
) -> Response {
    let mut request = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("cookie", cookie)
        .header("content-type", "application/json");
    if let Some(csrf) = csrf {
        request = request.header("x-csrf-token", csrf);
    }
    router(f.state.clone())
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}
impl Setup {
    async fn new(income: i64) -> Self {
        let db = TestDatabase::create().await;
        let owner = Database::connect(DatabaseConfig::new(db.owner_url.clone(), true))
            .await
            .unwrap();
        owner.migrate().await.unwrap();
        PostgresIdentityRepository::from_database(&owner)
            .grant_runtime_role(&db.runtime_role)
            .await
            .unwrap();
        PostgresCycleRepository::new(owner.clone())
            .grant_runtime_role(&db.runtime_role)
            .await
            .unwrap();
        PostgresEnvelopeRepository::new(owner.clone())
            .grant_runtime_role(&db.runtime_role)
            .await
            .unwrap();
        let runtime = Database::connect(DatabaseConfig::new(db.runtime_url.clone(), true))
            .await
            .unwrap();
        let mut fixture = Fixture::new(false, 300).await;
        fixture.state.users = Arc::new(PostgresIdentityRepository::from_database(&runtime));
        fixture.state.cycles = Some(obz_cycle::CycleService::new(Arc::new(
            PostgresCycleRepository::new(runtime.clone()),
        )));
        fixture.state.envelopes = Some(EnvelopeService::new(Arc::new(
            PostgresEnvelopeRepository::new(runtime.clone()),
        )));
        let (cookie, csrf) = fixture.authenticate().await;
        let user = me_data(&fixture, &cookie).await["user_id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let admin = PostgresIdentityRepository::from_database(&owner);
        admin.grant(user, WriteBudget).await.unwrap();
        let response = request(
            &fixture,
            &cookie,
            Some(&csrf),
            "POST",
            "/api/cycles",
            json!({"period_start":"2026-10-01","period_end":"2026-11-01","income_cents":income}),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let cycle = json(response).await["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        Self {
            db,
            owner,
            runtime,
            fixture,
            cookie,
            csrf,
            user,
            cycle,
        }
    }
    fn path(&self) -> String {
        format!("/api/cycles/{}/envelopes", self.cycle)
    }
    async fn write(&self, method: &str, path: &str, body: Value) -> Response {
        request(
            &self.fixture,
            &self.cookie,
            Some(&self.csrf),
            method,
            path,
            body,
        )
        .await
    }
    async fn read(&self, path: &str) -> Response {
        self.fixture
            .request("GET", path, Some(&self.cookie), None, None)
            .await
    }
    async fn grant_read(&self) {
        PostgresIdentityRepository::from_database(&self.owner)
            .grant(self.user, ReadBudget)
            .await
            .unwrap();
    }
    async fn balance(&self, allocated: i64, available: i64) {
        let data = json(self.read(&format!("/api/cycles/{}", self.cycle)).await).await;
        assert_eq!(data["allocated_cents"], allocated);
        assert_eq!(data["available_cents"], available);
        let data = json(self.read("/api/cycles").await).await;
        assert_eq!(data[0]["allocated_cents"], allocated);
        assert_eq!(data[0]["available_cents"], available);
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn envelopes_http_crud_validation_permissions_and_isolation() {
    let s = Setup::new(10000).await;
    let path = s.path();
    assert_eq!(
        s.fixture
            .request("GET", &path, None, None, None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(s.read(&path).await.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        request(
            &s.fixture,
            &s.cookie,
            None,
            "POST",
            &path,
            json!({"name":"Moradia","allocated_cents":6000})
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    // Write-only users can perform mutations without gaining public read access.
    let created = s
        .write(
            "POST",
            &path,
            json!({"name":" Moradia ","allocated_cents":6000}),
        )
        .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = json(created).await;
    assert_eq!(created["name"], "Moradia");
    let id = created["id"].as_str().unwrap();
    let item = format!("{path}/{id}");
    assert_eq!(s.read(&path).await.status(), StatusCode::FORBIDDEN);
    s.grant_read().await;
    s.balance(6000, 4000).await;
    assert_eq!(
        s.write("POST", &path, json!({"name":"Moradia","allocated_cents":0}))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        s.write(
            "POST",
            &path,
            json!({"name":"Excesso","allocated_cents":4001})
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    for body in [
        json!({"name":"","allocated_cents":0}),
        json!({"name":"x","allocated_cents":-1}),
        json!({"name":"x","allocated_cents":1.5}),
        json!({"name":"x","allocated_cents":0,"owner_id":s.user}),
        json!({"name":"x".repeat(101),"allocated_cents":0}),
        json!({"name":"a\nb","allocated_cents":0}),
    ] {
        assert_eq!(
            s.write("POST", &path, body).await.status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    for body in [
        json!({}),
        json!({"name":null}),
        json!({"allocated_cents":null}),
        json!({"cycle_id":s.cycle}),
        json!({"allocated_cents":-1}),
    ] {
        assert_eq!(
            s.write("PATCH", &item, body).await.status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(
        s.write(
            "POST",
            &path,
            json!({"name":"x".repeat(5000),"allocated_cents":0})
        )
        .await
        .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let second = s
        .write(
            "POST",
            &path,
            json!({"name":"Reserva","allocated_cents":4000}),
        )
        .await;
    assert_eq!(second.status(), StatusCode::CREATED);
    let second = json(second).await;
    let second_id = second["id"].as_str().unwrap();
    s.balance(10000, 0).await;
    assert_eq!(
        s.write("PATCH", &item, json!({"allocated_cents":6001}))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        s.write("PATCH", &item, json!({"name":"Reserva"}))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        s.write(
            "PATCH",
            &item,
            json!({"name":"Casa","allocated_cents":2000})
        )
        .await
        .status(),
        StatusCode::OK
    );
    s.balance(6000, 4000).await;
    let page = json(s.read(&format!("{path}?limit=1&offset=1")).await).await;
    assert_eq!(page.as_array().unwrap().len(), 1);
    assert_eq!(
        s.read(&format!("{path}?limit=101")).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        s.write("DELETE", &format!("{path}/{second_id}"), Value::Null)
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    s.balance(2000, 8000).await;
    assert_eq!(
        s.write("DELETE", &format!("{path}/{second_id}"), Value::Null)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        s.write(
            "PATCH",
            &format!("/api/cycles/{}/envelopes/{id}", Uuid::new_v4()),
            json!({"name":"Wrong"})
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let another = s
        .write(
            "POST",
            "/api/cycles",
            json!({"period_start":"2026-05-01","period_end":"2026-06-01","income_cents":10000}),
        )
        .await;
    assert_eq!(another.status(), StatusCode::CREATED);
    let another = json(another).await["id"].as_str().unwrap().to_owned();
    let wrong_parent = format!("/api/cycles/{another}/envelopes/{id}");
    assert_eq!(
        s.write("PATCH", &wrong_parent, json!({"name":"Wrong parent"}))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        s.write("DELETE", &wrong_parent, Value::Null).await.status(),
        StatusCode::NOT_FOUND
    );
    // A second, independently authenticated issuer produces a different local owner.
    let mut other = Fixture::new(false, 300).await;
    other.state.users = Arc::new(PostgresIdentityRepository::from_database(&s.runtime));
    other.state.cycles = s.fixture.state.cycles.clone();
    other.state.envelopes = s.fixture.state.envelopes.clone();
    let (other_cookie, other_csrf) = other.authenticate().await;
    let other_user: Uuid = me_data(&other, &other_cookie).await["user_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let admin = PostgresIdentityRepository::from_database(&s.owner);
    for permission in [ReadBudget, WriteBudget] {
        admin.grant(other_user, permission).await.unwrap();
    }
    for method in ["POST", "PATCH", "DELETE"] {
        let target = if method == "POST" {
            path.as_str()
        } else {
            item.as_str()
        };
        let body = if method == "POST" {
            json!({"name":"Other","allocated_cents":1})
        } else {
            json!({"name":"Other"})
        };
        assert_eq!(
            request(
                &other,
                &other_cookie,
                Some(&other_csrf),
                method,
                target,
                body
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        other
            .request("GET", &path, Some(&other_cookie), None, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    // RLS protects even SQL with the owner filter omitted.
    let mut tx = s.runtime.pool().begin().await.unwrap();
    sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
        .bind(other_user.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM budgeting.envelopes")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert!(sqlx::query("INSERT INTO budgeting.envelopes(id,cycle_id,owner_id,name,allocated_cents) VALUES($1,$2,$3,'forged',0)").bind(Uuid::new_v4()).bind(s.cycle).bind(s.user).execute(&mut *tx).await.is_err());
    tx.rollback().await.unwrap();
    for statement in [
        "UPDATE budgeting.envelopes SET owner_id=owner_id",
        "UPDATE budgeting.envelopes SET cycle_id=cycle_id",
        "UPDATE budgeting.envelopes SET id=id",
    ] {
        assert!(sqlx::query(statement)
            .execute(s.runtime.pool())
            .await
            .is_err());
    }
    admin.revoke(s.user, WriteBudget).await.unwrap();
    assert_eq!(
        s.write("PATCH", &item, json!({"allocated_cents":0}))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        s.write("DELETE", &item, Value::Null).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        s.write("POST", &path, json!({"name":"Denied","allocated_cents":0}))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    admin.revoke(s.user, ReadBudget).await.unwrap();
    assert_eq!(s.read(&path).await.status(), StatusCode::FORBIDDEN);
    s.runtime.pool().close().await;
    assert_eq!(
        s.read(&path).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        s.fixture
            .request(
                "POST",
                "/api/auth/logout",
                Some(&s.cookie),
                Some(&s.csrf),
                None
            )
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    s.db.destroy().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn envelope_allocation_serializes_concurrent_creates_updates_and_raw_sql() {
    let s = Setup::new(10000).await;
    s.grant_read().await;
    let user = LocalUser::from_persisted(s.user, [ReadBudget, WriteBudget].into_iter().collect());
    let service = s.fixture.state.envelopes.as_ref().unwrap().clone();
    let barrier = Arc::new(tokio::sync::Barrier::new(10));
    let mut tasks = tokio::task::JoinSet::new();
    for i in 0..10 {
        let service = service.clone();
        let user = user.clone();
        let barrier = barrier.clone();
        let cycle = s.cycle;
        tasks.spawn(async move {
            barrier.wait().await;
            service
                .create(&user, cycle, &format!("Concurrent{i}"), 2000)
                .await
        });
    }
    let mut ids = Vec::new();
    let mut denied = 0;
    while let Some(result) = tasks.join_next().await {
        match result.unwrap() {
            Ok(e) => ids.push(e.id),
            Err(CycleError::Overallocated) => denied += 1,
            Err(e) => panic!("{e:?}"),
        }
    }
    assert_eq!(ids.len(), 5);
    assert_eq!(denied, 5);
    s.balance(10000, 0).await;
    for id in ids {
        service.delete(&user, s.cycle, id).await.unwrap();
    }
    let a = service.create(&user, s.cycle, "A", 0).await.unwrap();
    let b = service.create(&user, s.cycle, "B", 0).await.unwrap();
    let (one, two) = tokio::join!(
        service.update(&user, s.cycle, a.id, None, Some(6000)),
        service.update(&user, s.cycle, b.id, None, Some(6000))
    );
    assert!(matches!(
        (&one, &two),
        (Ok(_), Err(CycleError::Overallocated)) | (Err(CycleError::Overallocated), Ok(_))
    ));
    s.balance(6000, 4000).await;
    service.delete(&user, s.cycle, a.id).await.unwrap();
    service.delete(&user, s.cycle, b.id).await.unwrap();
    // Bypass the adapter: the DB trigger must still serialize the limit check.
    async fn raw(db: Database, owner: Uuid, cycle: Uuid, name: &str) -> Result<(), sqlx::Error> {
        let mut tx = db.pool().begin().await?;
        sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
            .bind(owner.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO budgeting.envelopes(id,cycle_id,owner_id,name,allocated_cents) VALUES($1,$2,$3,$4,6000)").bind(Uuid::new_v4()).bind(cycle).bind(owner).bind(name).execute(&mut *tx).await?;
        tx.commit().await
    }
    let mut first = s.runtime.pool().begin().await.unwrap();
    sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
        .bind(s.user.to_string())
        .execute(&mut *first)
        .await
        .unwrap();
    sqlx::query("INSERT INTO budgeting.envelopes(id,cycle_id,owner_id,name,allocated_cents) VALUES($1,$2,$3,'RawA',6000)").bind(Uuid::new_v4()).bind(s.cycle).bind(s.user).execute(&mut *first).await.unwrap();
    let runtime = s.runtime.clone();
    let owner = s.user;
    let cycle = s.cycle;
    let second = tokio::spawn(async move { raw(runtime, owner, cycle, "RawB").await });
    tokio::time::timeout(std::time::Duration::from_secs(3),async {
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype='advisory' AND NOT granted AND database=(SELECT oid FROM pg_database WHERE datname=current_database()))").fetch_one(s.owner.pool()).await.unwrap();
            if waiting {break;}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("second write must wait for the first cycle allocation");
    first.commit().await.unwrap();
    let error = second.await.unwrap().unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("P0001")
    );
    s.balance(6000, 4000).await;
    // No per-user context survives the transaction on any reused pool connection.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM budgeting.envelopes")
        .fetch_one(s.runtime.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
    let mut tx = s.runtime.pool().begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
        .bind(s.user.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    let error=sqlx::query("INSERT INTO budgeting.envelopes(id,cycle_id,owner_id,name,allocated_cents) VALUES($1,$2,$3,'Unsupported snapshot',0)").bind(Uuid::new_v4()).bind(s.cycle).bind(s.user).execute(&mut *tx).await.unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("25000")
    );
    tx.rollback().await.unwrap();
    PostgresIdentityRepository::from_database(&s.owner)
        .revoke(s.user, WriteBudget)
        .await
        .unwrap();
    let stale = service.create(&user, s.cycle, "Revoked snapshot", 1).await;
    assert!(
        matches!(stale, Err(CycleError::Forbidden)),
        "RLS must reject a write even when the caller holds an old permission snapshot"
    );
    s.db.destroy().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn envelopes_support_zero_income_and_maximum_integer_without_overflow() {
    let s = Setup::new(i64::MAX).await;
    s.grant_read().await;
    let path = s.path();
    assert_eq!(
        s.write(
            "POST",
            &path,
            json!({"name":"Maximum","allocated_cents":i64::MAX})
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        s.write(
            "POST",
            &path,
            json!({"name":"One more","allocated_cents":1})
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    s.balance(i64::MAX, 0).await;
    let response = s
        .write(
            "POST",
            "/api/cycles",
            json!({"period_start":"2026-11-01","period_end":"2026-12-01","income_cents":0}),
        )
        .await;
    let zero = json(response).await["id"].as_str().unwrap().to_owned();
    let path = format!("/api/cycles/{zero}/envelopes");
    assert_eq!(
        s.write("POST", &path, json!({"name":"Zero","allocated_cents":0}))
            .await
            .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        s.write(
            "POST",
            &path,
            json!({"name":"Positive","allocated_cents":1})
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    s.db.destroy().await;
}
