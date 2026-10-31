use super::*;
use obz_banking::{CardInput, InvoiceInput, MovementInput, PaymentInput};
fn actor(s: &Setup) -> obz_auth::LocalUser {
    obz_auth::LocalUser::from_persisted(s.user, [ReadBudget, WriteBudget].into_iter().collect())
}
fn day(y: i32, m: u32, d: u32) -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()
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
async fn card(s: &Setup, limit: i64) -> Uuid {
    s.fixture
        .state
        .credit
        .as_ref()
        .unwrap()
        .create(
            &actor(s),
            CardInput {
                name: "Fixture card".into(),
                opened_on: day(1, 1, 1),
                credit_limit_cents: limit,
            },
        )
        .await
        .unwrap()
        .id
}
async fn invoice(s: &Setup, card: Uuid, month: u32) -> Uuid {
    s.fixture
        .state
        .credit
        .as_ref()
        .unwrap()
        .open(
            &actor(s),
            card,
            InvoiceInput {
                period_start: day(2026, month, 1),
                period_end: day(2026, month + 1, 1),
                due_on: day(2026, month + 1, 10),
            },
        )
        .await
        .unwrap()
        .id
}
async fn scope(s: &Setup, month: u32, amount: i64) -> (Uuid, Uuid) {
    let c = s
        .fixture
        .state
        .cycles
        .as_ref()
        .unwrap()
        .create(
            &actor(s),
            day(2026, month, 1),
            day(2026, month + 1, 1),
            amount,
        )
        .await
        .unwrap();
    let e = s
        .fixture
        .state
        .envelopes
        .as_ref()
        .unwrap()
        .create(&actor(s), c.id, "Card", amount)
        .await
        .unwrap();
    s.fixture
        .state
        .cycles
        .as_ref()
        .unwrap()
        .activate(&actor(s), c.id)
        .await
        .unwrap();
    (c.id, e.id)
}
async fn raw_purchase(
    s: &Setup,
    cycle: Uuid,
    envelope: Uuid,
    invoice: Uuid,
    amount: i64,
    date: chrono::NaiveDate,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    let mut tx = begin(s).await;
    let result=sqlx::query("INSERT INTO budgeting.expenses(id,owner_id,cycle_id,envelope_id,invoice_id,description,occurred_on,amount_cents) VALUES(gen_random_uuid(),$1,$2,$3,$4,'Credit',$5,$6)").bind(s.user).bind(cycle).bind(envelope).bind(invoice).bind(date).bind(amount).execute(&mut *tx).await;
    if result.is_ok() {
        tx.commit().await.unwrap();
    } else {
        tx.rollback().await.unwrap();
    }
    result
}
fn payment(s: &Setup, date: chrono::NaiveDate, amount: i64) -> PaymentInput {
    PaymentInput {
        request_id: Uuid::new_v4(),
        account_id: s.account,
        movement: MovementInput {
            description: "Bill".into(),
            occurred_on: date,
            amount_cents: amount,
        },
    }
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn credit_sql_limits_payments_immutability_and_exact_history() {
    let s = Setup::new(i64::MAX).await;
    s.grant_read().await;
    let card = card(&s, i64::MAX).await;
    let first = invoice(&s, card, 10).await;
    let second = invoice(&s, card, 11).await;
    let env = s
        .fixture
        .state
        .envelopes
        .as_ref()
        .unwrap()
        .create(&actor(&s), s.cycle, "First", i64::MAX)
        .await
        .unwrap();
    s.fixture
        .state
        .cycles
        .as_ref()
        .unwrap()
        .activate(&actor(&s), s.cycle)
        .await
        .unwrap();
    let (cycle2, env2) = scope(&s, 11, i64::MAX).await;
    raw_purchase(&s, s.cycle, env.id, first, i64::MAX, day(2026, 10, 1))
        .await
        .unwrap();
    assert_eq!(
        sqlstate(
            raw_purchase(&s, cycle2, env2, second, 1, day(2026, 11, 1))
                .await
                .unwrap_err()
        ),
        "ZC001"
    );
    let credit = s.fixture.state.credit.as_ref().unwrap();
    let mut tx = begin(&s).await;
    let open_payment=sqlx::query("INSERT INTO banking.card_payments(id,owner_id,invoice_id,account_id,description,occurred_on,amount_cents) VALUES(gen_random_uuid(),$1,$2,$3,'Open',DATE '2026-11-01',1)").bind(s.user).bind(first).bind(s.account).execute(&mut *tx).await;
    assert_eq!(sqlstate(open_payment.unwrap_err()), "ZC002");
    tx.rollback().await.unwrap();
    credit.close(&actor(&s), card, first).await.unwrap();
    let request = payment(&s, day(2026, 11, 1), i64::MAX);
    let settled = credit
        .pay(&actor(&s), card, first, request.clone())
        .await
        .unwrap();
    let replay = credit.pay(&actor(&s), card, first, request).await.unwrap();
    assert_eq!(settled.id, replay.id);
    raw_purchase(&s, cycle2, env2, second, i64::MAX, day(2026, 11, 1))
        .await
        .unwrap();
    assert_eq!(
        credit
            .get(&actor(&s), card)
            .await
            .unwrap()
            .outstanding
            .to_cents(),
        i64::MAX
    );
    s.fixture
        .state
        .banking
        .as_ref()
        .unwrap()
        .receipt(
            &actor(&s),
            s.account,
            MovementInput {
                description: "Fund next".into(),
                occurred_on: day(2026, 12, 1),
                amount_cents: i64::MAX,
            },
        )
        .await
        .unwrap();
    credit.close(&actor(&s), card, second).await.unwrap();
    credit
        .pay(
            &actor(&s),
            card,
            second,
            payment(&s, day(2026, 12, 1), i64::MAX),
        )
        .await
        .unwrap();
    assert_eq!(
        credit
            .get(&actor(&s), card)
            .await
            .unwrap()
            .outstanding
            .to_cents(),
        0
    );
    assert_eq!(
        s.fixture
            .state
            .banking
            .as_ref()
            .unwrap()
            .get(&actor(&s), s.account)
            .await
            .unwrap()
            .balance
            .to_cents(),
        0
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM banking.card_payments")
        .fetch_one(s.runtime.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
    let mut tx = begin(&s).await;
    let overpay=sqlx::query("INSERT INTO banking.card_payments(id,owner_id,invoice_id,account_id,description,occurred_on,amount_cents) VALUES(gen_random_uuid(),$1,$2,$3,'Overpay',DATE '2026-12-01',1)").bind(s.user).bind(second).bind(s.account).execute(&mut *tx).await;
    assert_eq!(sqlstate(overpay.unwrap_err()), "ZC004");
    tx.rollback().await.unwrap();
    let mut tx = begin(&s).await;
    let reopen = sqlx::query("UPDATE banking.card_invoices SET status='open' WHERE id=$1")
        .bind(first)
        .execute(&mut *tx)
        .await;
    assert_eq!(sqlstate(reopen.unwrap_err()), "ZC002");
    tx.rollback().await.unwrap();
    for statement in [
        "UPDATE banking.credit_cards SET credit_limit_cents=1",
        "DELETE FROM banking.credit_cards",
        "UPDATE banking.card_invoices SET due_on=due_on",
        "DELETE FROM banking.card_invoices",
        "UPDATE banking.card_payments SET amount_cents=1",
        "DELETE FROM banking.card_payments",
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
    let mut tx = begin(&s).await;
    let forged=sqlx::query("INSERT INTO banking.movements(id,owner_id,kind,from_account_id,description,occurred_on,amount_cents,card_payment_id) VALUES(gen_random_uuid(),$1,'card_payment',$2,'Forged',DATE '2026-12-01',1,$3)").bind(s.user).bind(s.account).bind(Uuid::new_v4()).execute(&mut *tx).await;
    assert_eq!(sqlstate(forged.unwrap_err()), "ZB004");
    tx.rollback().await.unwrap();
    s.db.destroy().await;
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL databases"]
async fn credit_raw_concurrent_purchases_observe_committed_global_limit() {
    let s = Setup::new(100).await;
    s.grant_read().await;
    let card = card(&s, 100).await;
    let first = invoice(&s, card, 10).await;
    let second = invoice(&s, card, 11).await;
    let e = s
        .fixture
        .state
        .envelopes
        .as_ref()
        .unwrap()
        .create(&actor(&s), s.cycle, "First", 100)
        .await
        .unwrap();
    s.fixture
        .state
        .cycles
        .as_ref()
        .unwrap()
        .activate(&actor(&s), s.cycle)
        .await
        .unwrap();
    let (cycle2, e2) = scope(&s, 11, 100).await;
    let mut first_tx = begin(&s).await;
    sqlx::query("INSERT INTO budgeting.expenses(id,owner_id,cycle_id,envelope_id,invoice_id,description,occurred_on,amount_cents) VALUES(gen_random_uuid(),$1,$2,$3,$4,'First',DATE '2026-10-01',100)").bind(s.user).bind(s.cycle).bind(e.id).bind(first).execute(&mut *first_tx).await.unwrap();
    let runtime = s.runtime.clone();
    let owner = s.user;
    let mut pending = tokio::spawn(async move {
        let mut tx = runtime.pool().begin().await.unwrap();
        sqlx::query("SELECT set_config('zerobudget.user_id',$1,true)")
            .bind(owner.to_string())
            .execute(&mut *tx)
            .await
            .unwrap();
        let result=sqlx::query("INSERT INTO budgeting.expenses(id,owner_id,cycle_id,envelope_id,invoice_id,description,occurred_on,amount_cents) VALUES(gen_random_uuid(),$1,$2,$3,$4,'Second',DATE '2026-11-01',100)").bind(owner).bind(cycle2).bind(e2).bind(second).execute(&mut *tx).await;
        tx.rollback().await.unwrap();
        result
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut pending)
            .await
            .is_err()
    );
    first_tx.commit().await.unwrap();
    assert_eq!(sqlstate(pending.await.unwrap().unwrap_err()), "ZC001");
    let credit = s.fixture.state.credit.as_ref().unwrap();
    assert_eq!(
        credit
            .get(&actor(&s), card)
            .await
            .unwrap()
            .outstanding
            .to_cents(),
        100
    );
    assert_eq!(
        credit
            .invoice(&actor(&s), card, second)
            .await
            .unwrap()
            .purchases
            .to_cents(),
        0
    );
    s.db.destroy().await;
}
