use super::*;
use obz_cycle::{CycleRepository, CycleTransition, ExpenseRepository};
use obz_infrastructure::PostgresExpenseRepository;

async fn envelope(s: &Setup, amount: i64) -> Uuid {
    let response = s
        .write(
            "POST",
            &s.path(),
            json!({"name":"Spending","allocated_cents":amount}),
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    json(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}
async fn tx(s: &Setup) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = s.runtime.pool().begin().await.unwrap();
    sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
        .bind(s.user.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx
}
async fn raw_expense(
    s: &Setup,
    envelope: Uuid,
    id: Uuid,
    amount: i64,
    date: &str,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    let mut tx = tx(s).await;
    let result=sqlx::query("INSERT INTO budgeting.expenses(id,cycle_id,owner_id,envelope_id,description,amount_cents,occurred_on,account_id) VALUES($1,$2,$3,$4,'Raw',$5,$6::text::date,$7)")
        .bind(id).bind(s.cycle).bind(s.user).bind(envelope).bind(amount).bind(date).bind(s.account).execute(&mut *tx).await;
    if result.is_ok() {
        tx.commit().await.unwrap();
    } else {
        tx.rollback().await.unwrap();
    }
    result
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn expenses_sql_guards_immutable_ledger_rls_dates_and_exact_i64_totals() {
    let s = Setup::new(i64::MAX).await;
    s.grant_read().await;
    let envelope = envelope(&s, i64::MAX).await;
    assert_eq!(
        sqlstate(
            raw_expense(&s, envelope, Uuid::new_v4(), 1, "2026-10-01")
                .await
                .unwrap_err()
        ),
        "ZB002"
    );
    let cycles = PostgresCycleRepository::new(s.runtime.clone());
    cycles
        .transition(s.user, s.cycle, CycleTransition::Activate)
        .await
        .unwrap();
    assert_eq!(
        sqlstate(
            raw_expense(&s, envelope, Uuid::new_v4(), 1, "2026-11-01")
                .await
                .unwrap_err()
        ),
        "ZB004"
    );
    assert_eq!(
        sqlstate(
            raw_expense(&s, envelope, Uuid::new_v4(), 0, "2026-10-01")
                .await
                .unwrap_err()
        ),
        "23514"
    );
    let mut batch_tx = tx(&s).await;
    let batch=sqlx::query("INSERT INTO budgeting.expenses(id,cycle_id,owner_id,envelope_id,description,amount_cents,occurred_on,account_id) VALUES(gen_random_uuid(),$1,$2,$3,'Batch A',$4,DATE '2026-10-01',(SELECT id FROM banking.accounts WHERE owner_id=$2 AND name='Fixture')),(gen_random_uuid(),$1,$2,$3,'Batch B',1,DATE '2026-10-01',(SELECT id FROM banking.accounts WHERE owner_id=$2 AND name='Fixture'))")
        .bind(s.cycle).bind(s.user).bind(envelope).bind(i64::MAX).execute(&mut *batch_tx).await;
    assert_eq!(sqlstate(batch.unwrap_err()), "ZB003");
    batch_tx.rollback().await.unwrap();
    let first = Uuid::new_v4();
    raw_expense(&s, envelope, first, i64::MAX, "2026-10-01")
        .await
        .unwrap();
    assert_eq!(
        sqlstate(
            raw_expense(&s, envelope, Uuid::new_v4(), 1, "2026-10-01")
                .await
                .unwrap_err()
        ),
        "ZB003"
    );
    let repository = PostgresExpenseRepository::new(s.runtime.clone());
    repository
        .reverse(s.user, s.cycle, envelope, first)
        .await
        .unwrap();
    raw_expense(&s, envelope, Uuid::new_v4(), i64::MAX, "2026-10-31")
        .await
        .unwrap();
    let data = json(s.read(&s.path()).await).await;
    assert_eq!(data[0]["spent_cents"], i64::MAX);
    assert_eq!(data[0]["remaining_cents"], 0);
    let records = repository
        .list(s.user, s.cycle, envelope, 20, 0)
        .await
        .unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records.iter().filter(|r| r.reversed).count(), 1);
    for statement in [
        "UPDATE budgeting.expenses SET amount_cents=1",
        "UPDATE budgeting.expenses SET owner_id=owner_id",
        "DELETE FROM budgeting.expenses",
        "UPDATE budgeting.expense_reversals SET expense_id=expense_id",
        "DELETE FROM budgeting.expense_reversals",
        "INSERT INTO budgeting.expenses(id,cycle_id,envelope_id,owner_id,description,occurred_on,amount_cents,created_at) VALUES(gen_random_uuid(),gen_random_uuid(),gen_random_uuid(),gen_random_uuid(),'x',CURRENT_DATE,1,now())",
    ] { assert_eq!(sqlstate(sqlx::query(statement).execute(s.runtime.pool()).await.unwrap_err()),"42501"); }
    // Use the authenticated context too, so this proves the trigger even when
    // the test database owner has CREATEDB without SUPERUSER/BYPASSRLS.
    for statement in [
        "UPDATE budgeting.expenses SET amount_cents=1",
        "DELETE FROM budgeting.expense_reversals",
    ] {
        let mut guard_tx = s.owner.pool().begin().await.unwrap();
        sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
            .bind(s.user.to_string())
            .execute(&mut *guard_tx)
            .await
            .unwrap();
        assert_eq!(
            sqlstate(
                sqlx::query(statement)
                    .execute(&mut *guard_tx)
                    .await
                    .unwrap_err()
            ),
            "42501"
        );
        guard_tx.rollback().await.unwrap();
    }
    let mut stale_tx = s.runtime.pool().begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *stale_tx)
        .await
        .unwrap();
    sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
        .bind(s.user.to_string())
        .execute(&mut *stale_tx)
        .await
        .unwrap();
    let stale=sqlx::query("INSERT INTO budgeting.expenses(id,cycle_id,envelope_id,owner_id,description,occurred_on,amount_cents,account_id) VALUES($1,$2,$3,$4,'Stale',DATE '2026-10-01',1,(SELECT id FROM banking.accounts WHERE owner_id=$4 AND name='Fixture'))")
        .bind(Uuid::new_v4()).bind(s.cycle).bind(envelope).bind(s.user).execute(&mut *stale_tx).await;
    assert_eq!(sqlstate(stale.unwrap_err()), "25000");
    stale_tx.rollback().await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM budgeting.expenses")
        .fetch_one(s.runtime.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
    let mut other = Fixture::new(false, 300).await;
    other.state.users = Arc::new(PostgresIdentityRepository::from_database(&s.runtime));
    let (cookie, _) = other.authenticate().await;
    let other_user: Uuid = me_data(&other, &cookie).await["user_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    PostgresIdentityRepository::from_database(&s.owner)
        .grant(other_user, WriteBudget)
        .await
        .unwrap();
    let mut other_tx = s.runtime.pool().begin().await.unwrap();
    sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
        .bind(other_user.to_string())
        .execute(&mut *other_tx)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM budgeting.expenses")
        .fetch_one(&mut *other_tx)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let forged=sqlx::query("INSERT INTO budgeting.expense_reversals(expense_id,cycle_id,envelope_id,owner_id) VALUES($1,$2,$3,$4)").bind(first).bind(s.cycle).bind(envelope).bind(s.user).execute(&mut *other_tx).await;
    assert_eq!(sqlstate(forged.unwrap_err()), "42501");
    other_tx.rollback().await.unwrap();
    cycles
        .transition(s.user, s.cycle, CycleTransition::Close)
        .await
        .unwrap();
    assert_eq!(
        sqlstate(
            raw_expense(&s, envelope, Uuid::new_v4(), 1, "2026-10-01")
                .await
                .unwrap_err()
        ),
        "ZB002"
    );
    let mut closed_tx = tx(&s).await;
    let closed_reverse=sqlx::query("INSERT INTO budgeting.expense_reversals(expense_id,cycle_id,envelope_id,owner_id) SELECT id,cycle_id,envelope_id,owner_id FROM budgeting.expenses WHERE id<>$1").bind(first).execute(&mut *closed_tx).await;
    assert_eq!(sqlstate(closed_reverse.unwrap_err()), "ZB002");
    closed_tx.rollback().await.unwrap();
    s.db.destroy().await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn expenses_raw_sql_and_closure_serialize_with_application_writes() {
    let s = Setup::new(100).await;
    s.grant_read().await;
    let envelope = envelope(&s, 100).await;
    let cycles = PostgresCycleRepository::new(s.runtime.clone());
    cycles
        .transition(s.user, s.cycle, CycleTransition::Activate)
        .await
        .unwrap();
    let first = Uuid::new_v4();
    let user = s.user;
    let cycle = s.cycle;
    let mut first_tx = tx(&s).await;
    sqlx::query("INSERT INTO budgeting.expenses(id,cycle_id,owner_id,envelope_id,description,amount_cents,occurred_on,account_id) VALUES($1,$2,$3,$4,'First',100,DATE '2026-10-01',(SELECT id FROM banking.accounts WHERE owner_id=$3 AND name='Fixture'))")
        .bind(first).bind(cycle).bind(user).bind(envelope).execute(&mut *first_tx).await.unwrap();
    let runtime = s.runtime.clone();
    let mut competing = tokio::spawn(async move {
        let mut tx = runtime.pool().begin().await.unwrap();
        sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
            .bind(user.to_string())
            .execute(&mut *tx)
            .await
            .unwrap();
        let result=sqlx::query("INSERT INTO budgeting.expenses(id,cycle_id,owner_id,envelope_id,description,amount_cents,occurred_on,account_id) VALUES($1,$2,$3,$4,'Second',100,DATE '2026-10-01',(SELECT id FROM banking.accounts WHERE owner_id=$3 AND name='Fixture'))")
            .bind(Uuid::new_v4()).bind(cycle).bind(user).bind(envelope).execute(&mut *tx).await;
        tx.rollback().await.unwrap();
        result
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut competing)
            .await
            .is_err()
    );
    first_tx.commit().await.unwrap();
    assert_eq!(sqlstate(competing.await.unwrap().unwrap_err()), "ZB003");
    let mut reverse_tx = tx(&s).await;
    sqlx::query("INSERT INTO budgeting.expense_reversals(expense_id,cycle_id,envelope_id,owner_id) VALUES($1,$2,$3,$4)").bind(first).bind(cycle).bind(envelope).bind(user).execute(&mut *reverse_tx).await.unwrap();
    let repository = PostgresExpenseRepository::new(s.runtime.clone());
    let account_id = s.account;
    let mut spend = tokio::spawn(async move {
        let user = obz_auth::LocalUser::from_persisted(user, [WriteBudget].into_iter().collect());
        obz_cycle::ExpenseService::new(Arc::new(repository))
            .create(
                &user,
                cycle,
                envelope,
                obz_cycle::ExpenseInput {
                    account_id,
                    description: "After reversal".into(),
                    date: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                    amount_cents: 100,
                },
            )
            .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut spend)
            .await
            .is_err()
    );
    reverse_tx.commit().await.unwrap();
    let posted = spend.await.unwrap().unwrap();
    let mut close_tx = tx(&s).await;
    sqlx::query("UPDATE budgeting.cycles SET status='closed' WHERE id=$1")
        .bind(cycle)
        .execute(&mut *close_tx)
        .await
        .unwrap();
    let repository = PostgresExpenseRepository::new(s.runtime.clone());
    let mut reversal =
        tokio::spawn(async move { repository.reverse(user, cycle, envelope, posted.id).await });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut reversal)
            .await
            .is_err()
    );
    close_tx.commit().await.unwrap();
    assert!(matches!(
        reversal.await.unwrap(),
        Err(CycleError::InvalidState)
    ));
    let data = json(s.read(&s.path()).await).await;
    assert_eq!(data[0]["spent_cents"], 100);
    assert_eq!(data[0]["remaining_cents"], 0);
    s.db.destroy().await;
}
