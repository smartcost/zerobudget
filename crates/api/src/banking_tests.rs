use super::*;
use obz_banking::{AccountInput, BankingRepository, MovementInput};
use obz_infrastructure::PostgresBankingRepository;
fn actor(s: &Setup) -> obz_auth::LocalUser {
    obz_auth::LocalUser::from_persisted(s.user, [ReadBudget, WriteBudget].into_iter().collect())
}
fn input(amount: i64) -> MovementInput {
    MovementInput {
        description: "Income".into(),
        occurred_on: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
        amount_cents: amount,
    }
}
async fn begin(s: &Setup) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = s.runtime.pool().begin().await.unwrap();
    sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
        .bind(s.user.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx
}
async fn target(s: &Setup) -> Uuid {
    s.fixture
        .state
        .banking
        .as_ref()
        .unwrap()
        .create(
            &actor(s),
            AccountInput {
                name: "Wallet".into(),
                kind: "cash".into(),
                opened_on: chrono::NaiveDate::from_ymd_opt(1, 1, 1).unwrap(),
                opening_balance_cents: 0,
            },
        )
        .await
        .unwrap()
        .id
}
async fn balance(s: &Setup, id: Uuid) -> i64 {
    s.fixture
        .state
        .banking
        .as_ref()
        .unwrap()
        .get(&actor(s), id)
        .await
        .unwrap()
        .balance
        .to_cents()
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn banking_sql_guards_funds_immutability_rls_and_integer_limits() {
    let s = Setup::new(i64::MAX).await;
    s.grant_read().await;
    let target = target(&s).await;
    let mut batch_tx = begin(&s).await;
    let batch=sqlx::query("INSERT INTO banking.movements(id,owner_id,kind,from_account_id,to_account_id,description,occurred_on,amount_cents) VALUES(uuidv7(),$1,'transfer',$2,$3,'Batch A',DATE '2026-10-01',$4),(uuidv7(),$1,'transfer',$2,$3,'Batch B',DATE '2026-10-01',1)")
        .bind(s.user).bind(s.account).bind(target).bind(i64::MAX).execute(&mut *batch_tx).await;
    assert_eq!(sqlstate(batch.unwrap_err()), "ZB005");
    batch_tx.rollback().await.unwrap();
    let mut tx = begin(&s).await;
    sqlx::query("INSERT INTO banking.movements(id,owner_id,kind,from_account_id,to_account_id,description,occurred_on,amount_cents) VALUES(uuidv7(),$1,'transfer',$2,$3,'All',DATE '2026-10-01',$4)").bind(s.user).bind(s.account).bind(target).bind(i64::MAX).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(balance(&s, s.account).await, 0);
    assert_eq!(balance(&s, target).await, i64::MAX);
    let mut tx = begin(&s).await;
    let excessive=sqlx::query("INSERT INTO banking.movements(id,owner_id,kind,to_account_id,description,occurred_on,amount_cents) VALUES(uuidv7(),$1,'receipt',$2,'Overflow',DATE '2026-10-01',1)").bind(s.user).bind(target).execute(&mut *tx).await;
    assert_eq!(sqlstate(excessive.unwrap_err()), "ZB006");
    tx.rollback().await.unwrap();
    let mut tx = begin(&s).await;
    let excessive=sqlx::query("INSERT INTO banking.movements(id,owner_id,kind,from_account_id,to_account_id,description,occurred_on,amount_cents) VALUES(uuidv7(),$1,'transfer',$2,$3,'Overdraft',DATE '2026-10-01',1)").bind(s.user).bind(s.account).bind(target).execute(&mut *tx).await;
    assert_eq!(sqlstate(excessive.unwrap_err()), "ZB005");
    tx.rollback().await.unwrap();
    s.fixture
        .state
        .banking
        .as_ref()
        .unwrap()
        .transfer(&actor(&s), target, s.account, input(i64::MAX))
        .await
        .unwrap();
    assert_eq!(balance(&s, s.account).await, i64::MAX);
    assert_eq!(balance(&s, target).await, 0);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM banking.movements")
        .fetch_one(s.runtime.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
    let forged=sqlx::query("INSERT INTO banking.accounts(id,owner_id,name,kind,opened_on,opening_balance_cents) VALUES(uuidv7(),$1,'Forged','cash',DATE '0001-01-01',0)").bind(s.user).execute(s.runtime.pool()).await;
    assert_eq!(sqlstate(forged.unwrap_err()), "42501");
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
    let stale=sqlx::query("INSERT INTO banking.movements(id,owner_id,kind,to_account_id,description,occurred_on,amount_cents) VALUES(uuidv7(),$1,'receipt',$2,'Stale',DATE '2026-10-01',1)").bind(s.user).bind(target).execute(&mut *stale_tx).await;
    assert_eq!(sqlstate(stale.unwrap_err()), "25000");
    stale_tx.rollback().await.unwrap();
    for statement in [
        "UPDATE banking.accounts SET opening_balance_cents=0",
        "DELETE FROM banking.accounts",
        "UPDATE banking.movements SET amount_cents=1",
        "DELETE FROM banking.movements",
    ] {
        assert_eq!(
            sqlstate(
                sqlx::query(statement)
                    .execute(s.runtime.pool())
                    .await
                    .unwrap_err()
            ),
            "42501"
        );
    }
    let mut tx = s.owner.pool().begin().await.unwrap();
    sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
        .bind(s.user.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        sqlstate(
            sqlx::query("UPDATE banking.movements SET amount_cents=1")
                .execute(&mut *tx)
                .await
                .unwrap_err()
        ),
        "42501"
    );
    tx.rollback().await.unwrap();
    // Direct SQL cannot forge a debit or refund without the matching budget record.
    let mut tx = begin(&s).await;
    let forged=sqlx::query("INSERT INTO banking.movements(id,owner_id,kind,to_account_id,description,occurred_on,amount_cents,expense_id) VALUES(uuidv7(),$1,'expense_reversal',$2,'Forged',DATE '2026-10-01',1,$3)").bind(s.user).bind(target).bind(Uuid::now_v7()).execute(&mut *tx).await;
    assert_eq!(sqlstate(forged.unwrap_err()), "ZB004");
    tx.rollback().await.unwrap();
    s.db.destroy().await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn banking_transfer_and_expense_share_funds_and_refund_failure_is_atomic() {
    let s = Setup::new(100).await;
    s.grant_read().await;
    let target = target(&s).await;
    let response = s
        .write(
            "POST",
            &s.path(),
            json!({"name":"Full","allocated_cents":100}),
        )
        .await;
    let envelope: Uuid = json(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    s.write(
        "POST",
        &format!("/api/cycles/{}/activate", s.cycle),
        Value::Null,
    )
    .await;
    let mut tx = begin(&s).await;
    sqlx::query("INSERT INTO banking.movements(id,owner_id,kind,from_account_id,to_account_id,description,occurred_on,amount_cents) VALUES(uuidv7(),$1,'transfer',$2,$3,'Race',DATE '2026-10-01',100)").bind(s.user).bind(s.account).bind(target).execute(&mut *tx).await.unwrap();
    let service = s.fixture.state.expenses.clone().unwrap();
    let user = actor(&s);
    let cycle = s.cycle;
    let account = s.account;
    let mut expense = tokio::spawn(async move {
        service
            .create(
                &user,
                cycle,
                envelope,
                obz_cycle::ExpenseInput {
                    funding: obz_cycle::ExpenseFunding::Account(account),
                    description: "Concurrent".into(),
                    date: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                    amount_cents: 100,
                },
            )
            .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut expense)
            .await
            .is_err()
    );
    tx.commit().await.unwrap();
    assert!(matches!(
        expense.await.unwrap(),
        Err(CycleError::AccountInsufficientFunds)
    ));
    s.balance(100, 0).await;
    s.fixture
        .state
        .banking
        .as_ref()
        .unwrap()
        .transfer(&actor(&s), target, s.account, input(100))
        .await
        .unwrap();
    let path = format!("/api/cycles/{}/envelopes/{envelope}/expenses", s.cycle);
    let response=s.write("POST",&path,json!({"account_id":account,"description":"Spend","occurred_on":"2026-10-01","amount_cents":100})).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let expense = json(response).await["id"].as_str().unwrap().to_owned();
    s.fixture
        .state
        .banking
        .as_ref()
        .unwrap()
        .receipt(&actor(&s), s.account, input(i64::MAX))
        .await
        .unwrap();
    assert_eq!(
        s.write("POST", &format!("{path}/{expense}/reverse"), Value::Null)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let records = json(s.read(&path).await).await;
    assert_eq!(records[0]["status"], "posted");
    assert_eq!(balance(&s, s.account).await, i64::MAX);
    let rows = json(s.read(&s.path()).await).await;
    assert_eq!(rows[0]["spent_cents"], 100);
    s.db.destroy().await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn banking_migration_preserves_unlinked_expenses_and_does_not_invent_balances() {
    let db = TestDatabase::create().await;
    let owner = Database::connect(DatabaseConfig::new(db.owner_url.clone(), true))
        .await
        .unwrap();
    for m in obz_infrastructure::MIGRATOR
        .iter()
        .filter(|m| m.version < 6)
    {
        sqlx::raw_sql(&m.sql).execute(owner.pool()).await.unwrap();
    }
    let user = Uuid::now_v7();
    let cycle = Uuid::now_v7();
    let envelope = Uuid::now_v7();
    let expense = Uuid::now_v7();
    let mut tx = owner.pool().begin().await.unwrap();
    sqlx::query(
        "INSERT INTO auth.users(id,issuer,subject) VALUES($1,'http://legacy.invalid','legacy')",
    )
    .bind(user)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("INSERT INTO auth.permissions VALUES($1,'budget:read'),($1,'budget:write')")
        .bind(user)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
        .bind(user.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO budgeting.cycles(id,owner_id,period_start,period_end,income_cents) VALUES($1,$2,DATE '2026-10-01',DATE '2026-11-01',100)").bind(cycle).bind(user).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO budgeting.envelopes(id,cycle_id,owner_id,name,allocated_cents) VALUES($1,$2,$3,'Legacy',100)").bind(envelope).bind(cycle).bind(user).execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE budgeting.cycles SET status='active' WHERE id=$1")
        .bind(cycle)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO budgeting.expenses(id,cycle_id,envelope_id,owner_id,description,occurred_on,amount_cents) VALUES($1,$2,$3,$4,'Legacy',DATE '2026-10-01',100)").bind(expense).bind(cycle).bind(envelope).bind(user).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let migration = obz_infrastructure::MIGRATOR
        .iter()
        .find(|m| m.version == 6)
        .unwrap();
    sqlx::raw_sql(&migration.sql)
        .execute(owner.pool())
        .await
        .unwrap();
    for m in obz_infrastructure::MIGRATOR
        .iter()
        .filter(|m| m.version > 6)
    {
        sqlx::raw_sql(&m.sql).execute(owner.pool()).await.unwrap();
    }
    obz_infrastructure::PostgresCreditRepository::new(owner.clone())
        .grant_runtime_role(&db.runtime_role)
        .await
        .unwrap();
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
    obz_infrastructure::PostgresExpenseRepository::new(owner.clone())
        .grant_runtime_role(&db.runtime_role)
        .await
        .unwrap();
    PostgresBankingRepository::new(owner.clone())
        .grant_runtime_role(&db.runtime_role)
        .await
        .unwrap();
    let runtime = Database::connect(DatabaseConfig::new(db.runtime_url.clone(), true))
        .await
        .unwrap();
    let repository = obz_infrastructure::PostgresExpenseRepository::new(runtime.clone());
    use obz_cycle::ExpenseRepository;
    let records = repository.list(user, cycle, envelope, 20, 0).await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].account_id, None);
    assert_eq!(records[0].expense.amount().to_cents(), 100);
    repository
        .reverse(user, cycle, envelope, expense)
        .await
        .unwrap();
    let movements = PostgresBankingRepository::new(runtime)
        .accounts(user, None, 20, 0)
        .await
        .unwrap();
    assert!(movements.is_empty());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM banking.movements")
        .fetch_one(owner.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
    db.destroy().await;
}
