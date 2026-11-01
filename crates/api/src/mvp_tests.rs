use super::*;
fn actor(s: &Setup) -> obz_auth::LocalUser {
    obz_auth::LocalUser::from_persisted(s.user, [ReadBudget, WriteBudget].into_iter().collect())
}
async fn tx(s: &Setup) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut t = s.runtime.pool().begin().await.unwrap();
    sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
        .bind(s.user.to_string())
        .execute(&mut *t)
        .await
        .unwrap();
    t
}
async fn envelopes(s: &Setup) -> (Uuid, Uuid) {
    let service = s.fixture.state.envelopes.as_ref().unwrap();
    let a = service
        .create(&actor(s), s.cycle, "Source", 70)
        .await
        .unwrap()
        .id;
    let b = service
        .create(&actor(s), s.cycle, "Target", 30)
        .await
        .unwrap()
        .id;
    (a, b)
}
async fn raw_move(s: &Setup, a: Uuid, b: Uuid, amount: i64) -> Result<(), sqlx::Error> {
    let mut t = tx(s).await;
    let result=sqlx::query("INSERT INTO budgeting.reallocations(id,owner_id,cycle_id,from_envelope_id,to_envelope_id,amount_cents) VALUES(uuidv7(),$1,$2,$3,$4,$5)").bind(s.user).bind(s.cycle).bind(a).bind(b).bind(amount).execute(&mut *t).await;
    match result {
        Ok(_) => t.commit().await,
        Err(e) => {
            t.rollback().await.unwrap();
            Err(e)
        }
    }
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn raw_reallocation_serializes_and_preserves_spent_and_immutable_history() {
    let s = Setup::new(100).await;
    s.grant_read().await;
    let (a, b) = envelopes(&s).await;
    assert_eq!(sqlstate(raw_move(&s, a, b, 10).await.unwrap_err()), "ZB002");
    s.fixture
        .state
        .cycles
        .as_ref()
        .unwrap()
        .activate(&actor(&s), s.cycle)
        .await
        .unwrap();
    s.fixture
        .state
        .expenses
        .as_ref()
        .unwrap()
        .create(
            &actor(&s),
            s.cycle,
            a,
            obz_cycle::ExpenseInput {
                funding: obz_cycle::ExpenseFunding::Account(s.account),
                description: "Spent".into(),
                date: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                amount_cents: 50,
            },
        )
        .await
        .unwrap();
    let (x, y) = tokio::join!(raw_move(&s, a, b, 15), raw_move(&s, a, b, 15));
    assert_eq!(usize::from(x.is_ok()) + usize::from(y.is_ok()), 1);
    assert_eq!(sqlstate(x.err().or(y.err()).unwrap()), "ZB003");
    let mut t = tx(&s).await;
    assert_eq!(
        sqlstate(
            sqlx::query("UPDATE budgeting.reallocations SET amount_cents=1 WHERE owner_id=$1")
                .bind(s.user)
                .execute(&mut *t)
                .await
                .unwrap_err()
        ),
        "42501"
    );
    t.rollback().await.unwrap();
    let summary = s
        .fixture
        .state
        .planning
        .as_ref()
        .unwrap()
        .summary(&actor(&s), s.cycle)
        .await
        .unwrap();
    assert_eq!(summary.spent_cents, 50);
    assert_eq!(summary.remaining_cents, 50);
    s.db.destroy().await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn raw_corrections_are_linked_atomic_and_owner_isolated() {
    let s = Setup::new(100).await;
    s.grant_read().await;
    let service = s.fixture.state.banking.as_ref().unwrap();
    let user = actor(&s);
    let m = service
        .receipt(
            &user,
            s.account,
            obz_banking::MovementInput {
                description: "Receipt".into(),
                occurred_on: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                amount_cents: 10,
            },
        )
        .await
        .unwrap();
    let mut t = tx(&s).await;
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO banking.adjustments(id,owner_id,movement_id,description,occurred_on) VALUES($1,$2,$3,'Correction',DATE '2026-10-02')").bind(id).bind(s.user).bind(m.id).execute(&mut *t).await.unwrap();
    t.commit().await.unwrap();
    assert_eq!(
        service
            .get(&user, s.account)
            .await
            .unwrap()
            .balance
            .to_cents(),
        100
    );
    let mut t = tx(&s).await;
    assert_eq!(sqlstate(sqlx::query("INSERT INTO banking.movements(id,owner_id,kind,from_account_id,description,occurred_on,amount_cents,adjustment_id) VALUES(uuidv7(),$1,'correction',$2,'Forged',DATE '2026-10-02',1,$3)").bind(s.user).bind(s.account).bind(id).execute(&mut *t).await.unwrap_err()),"ZB004");
    t.rollback().await.unwrap();
    let mut t = tx(&s).await;
    assert_eq!(
        sqlstate(
            sqlx::query("DELETE FROM banking.adjustments WHERE owner_id=$1")
                .bind(s.user)
                .execute(&mut *t)
                .await
                .unwrap_err()
        ),
        "42501"
    );
    t.rollback().await.unwrap();
    let mut t = s.runtime.pool().begin().await.unwrap();
    sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
        .bind(Uuid::now_v7().to_string())
        .execute(&mut *t)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM banking.adjustments")
        .fetch_one(&mut *t)
        .await
        .unwrap();
    assert_eq!(count, 0);
    t.rollback().await.unwrap();
    s.db.destroy().await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn summaries_and_readiness_fail_closed_when_database_disappears() {
    let mut s = Setup::new(0).await;
    s.fixture.state = s.fixture.state.clone().with_database(s.runtime.clone());
    assert_eq!(s.read("/ready").await.status(), StatusCode::NO_CONTENT);
    s.runtime.pool().close().await;
    assert_eq!(
        s.read("/ready").await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    s.db.destroy().await;
}
