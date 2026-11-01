ALTER TABLE banking.movements ADD CONSTRAINT movements_id_owner_key UNIQUE(id,owner_id);
CREATE TABLE banking.adjustments (
 id uuid PRIMARY KEY,owner_id uuid NOT NULL,movement_id uuid NOT NULL UNIQUE,
 description text NOT NULL CHECK(char_length(description) BETWEEN 1 AND 200 AND description=btrim(description) AND description !~ '[[:cntrl:]]'),
 occurred_on date NOT NULL CHECK(occurred_on BETWEEN DATE '0001-01-01' AND DATE '9999-12-31'),created_at timestamptz NOT NULL DEFAULT now(),
 FOREIGN KEY(movement_id,owner_id) REFERENCES banking.movements(id,owner_id),UNIQUE(id,owner_id)
);
CREATE TABLE banking.opening_debts (
 id uuid PRIMARY KEY,owner_id uuid NOT NULL,card_id uuid NOT NULL,invoice_id uuid NOT NULL,
 description text NOT NULL CHECK(char_length(description) BETWEEN 1 AND 200 AND description=btrim(description) AND description !~ '[[:cntrl:]]'),
 occurred_on date NOT NULL,amount_cents bigint NOT NULL CHECK(amount_cents>0),created_at timestamptz NOT NULL DEFAULT now(),
 FOREIGN KEY(card_id,owner_id) REFERENCES banking.credit_cards(id,owner_id),FOREIGN KEY(invoice_id,owner_id) REFERENCES banking.card_invoices(id,owner_id),UNIQUE(id,owner_id)
);
CREATE TABLE banking.card_credits (
 id uuid PRIMARY KEY,owner_id uuid NOT NULL,card_id uuid NOT NULL,invoice_id uuid NOT NULL,
 expense_id uuid UNIQUE,opening_debt_id uuid UNIQUE,
 description text NOT NULL CHECK(char_length(description) BETWEEN 1 AND 200 AND description=btrim(description) AND description !~ '[[:cntrl:]]'),
 occurred_on date NOT NULL,amount_cents bigint NOT NULL CHECK(amount_cents>0),created_at timestamptz NOT NULL DEFAULT now(),
 CHECK(num_nonnulls(expense_id,opening_debt_id)=1),
 FOREIGN KEY(card_id,owner_id) REFERENCES banking.credit_cards(id,owner_id),FOREIGN KEY(invoice_id,owner_id) REFERENCES banking.card_invoices(id,owner_id),
 FOREIGN KEY(expense_id,owner_id) REFERENCES budgeting.expenses(id,owner_id),FOREIGN KEY(opening_debt_id,owner_id) REFERENCES banking.opening_debts(id,owner_id)
);
ALTER TABLE banking.movements ADD COLUMN adjustment_id uuid;
ALTER TABLE banking.movements ADD CONSTRAINT movements_adjustment_fk FOREIGN KEY(adjustment_id,owner_id) REFERENCES banking.adjustments(id,owner_id);
ALTER TABLE banking.movements ADD CONSTRAINT movements_adjustment_key UNIQUE(adjustment_id);
ALTER TABLE banking.movements DROP CONSTRAINT movements_kind_check;
ALTER TABLE banking.movements DROP CONSTRAINT movements_check;
ALTER TABLE banking.movements ADD CONSTRAINT movements_kind_check CHECK(kind IN('receipt','transfer','expense','expense_reversal','card_payment','correction'));
ALTER TABLE banking.movements ADD CONSTRAINT movements_check CHECK(
 (((kind='receipt' AND from_account_id IS NULL AND to_account_id IS NOT NULL AND expense_id IS NULL)
 OR(kind='transfer' AND from_account_id IS NOT NULL AND to_account_id IS NOT NULL AND from_account_id<>to_account_id AND expense_id IS NULL)
 OR(kind='expense' AND from_account_id IS NOT NULL AND to_account_id IS NULL AND expense_id IS NOT NULL)
 OR(kind='expense_reversal' AND from_account_id IS NULL AND to_account_id IS NOT NULL AND expense_id IS NOT NULL)) AND card_payment_id IS NULL
 OR(kind='card_payment' AND from_account_id IS NOT NULL AND to_account_id IS NULL AND expense_id IS NULL AND card_payment_id IS NOT NULL)) AND adjustment_id IS NULL
 OR(kind='correction' AND num_nonnulls(from_account_id,to_account_id)>0 AND from_account_id IS DISTINCT FROM to_account_id AND expense_id IS NULL AND card_payment_id IS NULL AND adjustment_id IS NOT NULL)
);

ALTER TABLE banking.adjustments ENABLE ROW LEVEL SECURITY;
ALTER TABLE banking.adjustments FORCE ROW LEVEL SECURITY;
CREATE POLICY adjustments_select ON banking.adjustments FOR SELECT USING(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission IN('budget:read','budget:write')));
CREATE POLICY adjustments_insert ON banking.adjustments FOR INSERT WITH CHECK(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write'));
CREATE TRIGGER immutable_adjustments BEFORE UPDATE OR DELETE ON banking.adjustments FOR EACH ROW EXECUTE FUNCTION banking.immutable();

ALTER TABLE banking.opening_debts ENABLE ROW LEVEL SECURITY;
ALTER TABLE banking.opening_debts FORCE ROW LEVEL SECURITY;
CREATE POLICY opening_debts_select ON banking.opening_debts FOR SELECT USING(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission IN('budget:read','budget:write')));
CREATE POLICY opening_debts_insert ON banking.opening_debts FOR INSERT WITH CHECK(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write'));
CREATE TRIGGER immutable_opening_debts BEFORE UPDATE OR DELETE ON banking.opening_debts FOR EACH ROW EXECUTE FUNCTION banking.immutable();

ALTER TABLE banking.card_credits ENABLE ROW LEVEL SECURITY;
ALTER TABLE banking.card_credits FORCE ROW LEVEL SECURITY;
CREATE POLICY card_credits_select ON banking.card_credits FOR SELECT USING(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission IN('budget:read','budget:write')));
CREATE POLICY card_credits_insert ON banking.card_credits FOR INSERT WITH CHECK(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write'));
CREATE TRIGGER immutable_card_credits BEFORE UPDATE OR DELETE ON banking.card_credits FOR EACH ROW EXECUTE FUNCTION banking.immutable();

CREATE VIEW banking.invoice_totals WITH(security_invoker=true) AS
 SELECT i.id,i.owner_id,i.card_id,
 COALESCE((SELECT sum(x.amount_cents::numeric) FROM budgeting.expenses x WHERE x.invoice_id=i.id AND x.owner_id=i.owner_id AND NOT EXISTS(SELECT 1 FROM budgeting.expense_reversals r WHERE r.expense_id=x.id)),0) AS purchases,
 COALESCE((SELECT sum(d.amount_cents::numeric) FROM banking.opening_debts d WHERE d.invoice_id=i.id AND d.owner_id=i.owner_id),0) AS opening_debt,
 COALESCE((SELECT sum(c.amount_cents::numeric) FROM banking.card_credits c WHERE c.invoice_id=i.id AND c.owner_id=i.owner_id),0) AS credits,
 COALESCE((SELECT sum(p.amount_cents::numeric) FROM banking.card_payments p WHERE p.invoice_id=i.id AND p.owner_id=i.owner_id AND NOT EXISTS(SELECT 1 FROM banking.movements m JOIN banking.adjustments a ON a.movement_id=m.id AND a.owner_id=m.owner_id WHERE m.card_payment_id=p.id AND m.owner_id=p.owner_id)),0) AS paid
 FROM banking.card_invoices i;
CREATE VIEW banking.invoice_positions WITH(security_invoker=true) AS SELECT t.*,t.purchases+t.opening_debt-t.credits-t.paid AS debt FROM banking.invoice_totals t;
CREATE VIEW banking.card_positions WITH(security_invoker=true) AS SELECT c.id,c.owner_id,COALESCE((SELECT sum(i.debt) FROM banking.invoice_positions i WHERE i.card_id=c.id AND i.owner_id=c.owner_id),0) AS debt FROM banking.credit_cards c;
CREATE FUNCTION banking.guard_adjustment() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE original record;card uuid;position numeric;maximum bigint;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' THEN RAISE EXCEPTION 'corrections require read committed' USING ERRCODE='25000'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('zerobudget.banking:'||NEW.owner_id::text,0));
 SELECT * INTO original FROM banking.movements WHERE id=NEW.movement_id AND owner_id=NEW.owner_id;
 IF NOT FOUND THEN RAISE EXCEPTION 'movement unavailable' USING ERRCODE='42501'; END IF;
 IF original.kind NOT IN('receipt','transfer','card_payment') OR NEW.occurred_on<original.occurred_on THEN RAISE EXCEPTION 'invalid correction target/date' USING ERRCODE='ZB004'; END IF;
 IF original.kind='card_payment' THEN
  SELECT i.card_id INTO card FROM banking.card_payments p JOIN banking.card_invoices i ON i.id=p.invoice_id AND i.owner_id=p.owner_id WHERE p.id=original.card_payment_id AND p.owner_id=NEW.owner_id;
  SELECT p.debt,c.credit_limit_cents INTO position,maximum FROM banking.card_positions p JOIN banking.credit_cards c ON c.id=p.id AND c.owner_id=p.owner_id WHERE p.id=card AND p.owner_id=NEW.owner_id;
  IF position+original.amount_cents>maximum THEN RAISE EXCEPTION 'payment reversal exceeds credit limit' USING ERRCODE='ZC001'; END IF;
 END IF;
 RETURN NEW;
END; $$;
REVOKE ALL ON FUNCTION banking.guard_adjustment() FROM PUBLIC;
CREATE TRIGGER guard_adjustment BEFORE INSERT ON banking.adjustments FOR EACH ROW EXECUTE FUNCTION banking.guard_adjustment();
CREATE FUNCTION banking.record_adjustment() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE original record;
BEGIN
 SELECT * INTO original FROM banking.movements WHERE id=NEW.movement_id AND owner_id=NEW.owner_id;
 INSERT INTO banking.movements(id,owner_id,kind,from_account_id,to_account_id,description,occurred_on,amount_cents,adjustment_id) VALUES(gen_random_uuid(),NEW.owner_id,'correction',original.to_account_id,original.from_account_id,NEW.description,NEW.occurred_on,original.amount_cents,NEW.id);
 RETURN NEW;
END; $$;
REVOKE ALL ON FUNCTION banking.record_adjustment() FROM PUBLIC;
CREATE TRIGGER record_adjustment AFTER INSERT ON banking.adjustments FOR EACH ROW EXECUTE FUNCTION banking.record_adjustment();
CREATE FUNCTION banking.guard_credit_entry() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE bill record;source record;position numeric;gross numeric;maximum bigint;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' THEN RAISE EXCEPTION 'credit adjustments require read committed' USING ERRCODE='25000'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('zerobudget.banking:'||NEW.owner_id::text,0));
 SELECT i.status,i.period_start,i.period_end,c.opened_on,c.credit_limit_cents INTO bill FROM banking.card_invoices i JOIN banking.credit_cards c ON c.id=i.card_id AND c.owner_id=i.owner_id WHERE i.id=NEW.invoice_id AND i.card_id=NEW.card_id AND i.owner_id=NEW.owner_id;
 IF NOT FOUND THEN RAISE EXCEPTION 'invoice unavailable' USING ERRCODE='42501'; END IF;
 IF bill.status<>'open' THEN RAISE EXCEPTION 'target invoice is closed' USING ERRCODE='ZC002'; END IF;
 IF NEW.occurred_on<bill.period_start OR NEW.occurred_on>=bill.period_end OR NEW.occurred_on<bill.opened_on THEN RAISE EXCEPTION 'entry outside invoice period' USING ERRCODE='ZB004'; END IF;
 SELECT debt INTO position FROM banking.card_positions WHERE id=NEW.card_id AND owner_id=NEW.owner_id;
 IF TG_TABLE_NAME='opening_debts' THEN
  IF EXISTS(SELECT 1 FROM banking.opening_debts d WHERE d.owner_id=NEW.owner_id AND d.card_id=NEW.card_id AND NOT EXISTS(SELECT 1 FROM banking.card_credits c WHERE c.opening_debt_id=d.id)) THEN RAISE EXCEPTION 'opening debt already entered' USING ERRCODE='ZC003'; END IF;
  IF position+NEW.amount_cents>bill.credit_limit_cents THEN RAISE EXCEPTION 'credit limit exceeded' USING ERRCODE='ZC001'; END IF;
  SELECT purchases+opening_debt INTO gross FROM banking.invoice_positions WHERE id=NEW.invoice_id AND owner_id=NEW.owner_id;
  IF gross+NEW.amount_cents>9223372036854775807 THEN RAISE EXCEPTION 'invoice total overflow' USING ERRCODE='ZB006'; END IF;
 ELSE
  IF NEW.expense_id IS NOT NULL THEN
   SELECT x.amount_cents,x.occurred_on,i.status,i.card_id,EXISTS(SELECT 1 FROM budgeting.expense_reversals r WHERE r.expense_id=x.id) AS reversed INTO source FROM budgeting.expenses x JOIN banking.card_invoices i ON i.id=x.invoice_id AND i.owner_id=x.owner_id WHERE x.id=NEW.expense_id AND x.owner_id=NEW.owner_id;
   IF NOT FOUND OR source.status<>'closed' OR source.reversed OR source.card_id<>NEW.card_id THEN RAISE EXCEPTION 'invalid closed purchase refund' USING ERRCODE='ZB004'; END IF;
  ELSE
   SELECT amount_cents,occurred_on,card_id INTO source FROM banking.opening_debts WHERE id=NEW.opening_debt_id AND owner_id=NEW.owner_id;
   IF NOT FOUND OR source.card_id<>NEW.card_id THEN RAISE EXCEPTION 'invalid opening debt correction' USING ERRCODE='ZB004'; END IF;
  END IF;
  IF NEW.amount_cents<>source.amount_cents OR NEW.occurred_on<source.occurred_on THEN RAISE EXCEPTION 'refund amount/date mismatch' USING ERRCODE='ZB004'; END IF;
  SELECT credits INTO gross FROM banking.invoice_positions WHERE id=NEW.invoice_id AND owner_id=NEW.owner_id;
  IF position-NEW.amount_cents < -9223372036854775807 OR gross+NEW.amount_cents>9223372036854775807 THEN RAISE EXCEPTION 'credit balance overflow' USING ERRCODE='ZB006'; END IF;
 END IF;
 RETURN NEW;
END; $$;
REVOKE ALL ON FUNCTION banking.guard_credit_entry() FROM PUBLIC;
CREATE TRIGGER guard_opening_debt BEFORE INSERT ON banking.opening_debts FOR EACH ROW EXECUTE FUNCTION banking.guard_credit_entry();
CREATE TRIGGER guard_card_credit BEFORE INSERT ON banking.card_credits FOR EACH ROW EXECUTE FUNCTION banking.guard_credit_entry();

CREATE OR REPLACE FUNCTION banking.guard_movement() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE a record; balance numeric; next_balance numeric; payment record;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' THEN RAISE EXCEPTION 'banking writes require read committed' USING ERRCODE='25000'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('zerobudget.banking:'||NEW.owner_id::text,0));
 IF NEW.kind='correction' THEN
   SELECT adj.description,adj.occurred_on,m.amount_cents,m.from_account_id,m.to_account_id INTO payment FROM banking.adjustments adj JOIN banking.movements m ON m.id=adj.movement_id AND m.owner_id=adj.owner_id WHERE adj.id=NEW.adjustment_id AND adj.owner_id=NEW.owner_id;
   IF NOT FOUND OR NEW.from_account_id IS DISTINCT FROM payment.to_account_id OR NEW.to_account_id IS DISTINCT FROM payment.from_account_id OR NEW.amount_cents<>payment.amount_cents OR NEW.description<>payment.description OR NEW.occurred_on<>payment.occurred_on THEN RAISE EXCEPTION 'invalid compensation movement' USING ERRCODE='ZB004'; END IF;
 END IF;
 IF NEW.kind='card_payment' THEN
   SELECT account_id,amount_cents,occurred_on,description INTO payment FROM banking.card_payments WHERE id=NEW.card_payment_id AND owner_id=NEW.owner_id;
   IF NOT FOUND OR payment.account_id<>NEW.from_account_id OR payment.amount_cents<>NEW.amount_cents OR payment.occurred_on<>NEW.occurred_on OR payment.description<>NEW.description THEN RAISE EXCEPTION 'invalid card settlement' USING ERRCODE='ZB004'; END IF;
 END IF;
 IF NEW.kind IN('expense','expense_reversal') THEN
   SELECT x.account_id,x.amount_cents,x.occurred_on,x.description INTO payment FROM budgeting.expenses x WHERE x.id=NEW.expense_id AND x.owner_id=NEW.owner_id;
   IF NOT FOUND OR payment.account_id IS NULL OR payment.account_id<>COALESCE(NEW.from_account_id,NEW.to_account_id) OR payment.amount_cents<>NEW.amount_cents OR payment.occurred_on<>NEW.occurred_on OR payment.description<>NEW.description THEN RAISE EXCEPTION 'invalid expense payment' USING ERRCODE='ZB004'; END IF;
   IF NEW.kind='expense_reversal' AND NOT EXISTS(SELECT 1 FROM budgeting.expense_reversals r WHERE r.expense_id=NEW.expense_id AND r.owner_id=NEW.owner_id) THEN RAISE EXCEPTION 'reversal marker missing' USING ERRCODE='ZB004'; END IF;
 END IF;
 FOR a IN SELECT id,opened_on,opening_balance_cents FROM banking.accounts WHERE owner_id=NEW.owner_id AND id IN(NEW.from_account_id,NEW.to_account_id) ORDER BY (id=NEW.from_account_id) DESC NULLS LAST,id LOOP
   IF NEW.occurred_on<a.opened_on THEN RAISE EXCEPTION 'movement before opening date' USING ERRCODE='ZB004'; END IF;
   SELECT a.opening_balance_cents+COALESCE(sum(CASE WHEN m.to_account_id=a.id THEN m.amount_cents::numeric ELSE -m.amount_cents::numeric END),0) INTO balance FROM banking.movements m WHERE m.owner_id=NEW.owner_id AND (m.from_account_id=a.id OR m.to_account_id=a.id);
   next_balance:=balance+CASE WHEN NEW.to_account_id=a.id THEN NEW.amount_cents::numeric ELSE -NEW.amount_cents::numeric END;
   IF next_balance<0 THEN RAISE EXCEPTION 'insufficient account balance' USING ERRCODE='ZB005'; END IF;
   IF next_balance>9223372036854775807 THEN RAISE EXCEPTION 'account balance overflow' USING ERRCODE='ZB006'; END IF;
 END LOOP;
 RETURN NEW;
END; $$;
REVOKE ALL ON FUNCTION banking.guard_movement() FROM PUBLIC;


CREATE OR REPLACE FUNCTION banking.guard_credit_expense() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE invoice uuid;bill record;debt numeric;original_amount bigint;
BEGIN
 IF TG_TABLE_NAME='expenses' THEN invoice:=NEW.invoice_id;
 ELSE SELECT invoice_id INTO invoice FROM budgeting.expenses WHERE id=NEW.expense_id AND owner_id=NEW.owner_id;
 END IF;
 IF invoice IS NULL THEN RETURN NEW; END IF;
 IF current_setting('transaction_isolation')<>'read committed' THEN RAISE EXCEPTION 'credit writes require read committed' USING ERRCODE='25000'; END IF;
 -- guard_expense_insert runs first and holds the cycle lock; never reverse order.
 PERFORM pg_advisory_xact_lock(hashtextextended('zerobudget.banking:'||NEW.owner_id::text,0));
 SELECT i.status,i.period_start,i.period_end,i.card_id,c.opened_on,c.credit_limit_cents INTO bill FROM banking.card_invoices i JOIN banking.credit_cards c ON c.id=i.card_id AND c.owner_id=i.owner_id WHERE i.id=invoice AND i.owner_id=NEW.owner_id;
 IF NOT FOUND THEN RAISE EXCEPTION 'invoice unavailable' USING ERRCODE='42501'; END IF;
 IF bill.status<>'open' THEN RAISE EXCEPTION 'invoice is closed' USING ERRCODE='ZC002'; END IF;
 IF TG_TABLE_NAME='expenses' THEN
  IF NEW.occurred_on<bill.period_start OR NEW.occurred_on>=bill.period_end OR NEW.occurred_on<bill.opened_on THEN RAISE EXCEPTION 'purchase outside invoice period' USING ERRCODE='ZB004'; END IF;
  SELECT p.debt INTO debt FROM banking.card_positions p WHERE p.id=bill.card_id AND p.owner_id=NEW.owner_id;
  IF (SELECT purchases+opening_debt FROM banking.invoice_positions WHERE id=invoice AND owner_id=NEW.owner_id)+NEW.amount_cents>9223372036854775807 THEN RAISE EXCEPTION 'invoice total overflow' USING ERRCODE='ZB006'; END IF;
  IF debt+NEW.amount_cents>bill.credit_limit_cents THEN RAISE EXCEPTION 'credit limit exceeded' USING ERRCODE='ZC001'; END IF;
 ELSE
  SELECT p.debt INTO debt FROM banking.card_positions p WHERE p.id=bill.card_id AND p.owner_id=NEW.owner_id;
  SELECT amount_cents INTO original_amount FROM budgeting.expenses WHERE id=NEW.expense_id AND owner_id=NEW.owner_id;
  IF debt-original_amount < -9223372036854775807 THEN RAISE EXCEPTION 'credit balance overflow' USING ERRCODE='ZB006'; END IF;
 END IF;
 RETURN NEW;
END; $$;
REVOKE ALL ON FUNCTION banking.guard_credit_expense() FROM PUBLIC;

CREATE OR REPLACE FUNCTION banking.guard_card_payment() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE bill record;remaining numeric;position numeric;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' THEN RAISE EXCEPTION 'credit writes require read committed' USING ERRCODE='25000'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('zerobudget.banking:'||NEW.owner_id::text,0));
 SELECT status,period_end,card_id INTO bill FROM banking.card_invoices WHERE id=NEW.invoice_id AND owner_id=NEW.owner_id;
 IF NOT FOUND THEN RAISE EXCEPTION 'invoice unavailable' USING ERRCODE='42501'; END IF;
 IF bill.status<>'closed' THEN RAISE EXCEPTION 'invoice must be closed' USING ERRCODE='ZC002'; END IF;
 IF NEW.occurred_on<bill.period_end THEN RAISE EXCEPTION 'payment precedes invoice period end' USING ERRCODE='ZB004'; END IF;
 SELECT greatest(debt,0) INTO remaining FROM banking.invoice_positions WHERE id=NEW.invoice_id AND owner_id=NEW.owner_id;
 SELECT p.debt INTO position FROM banking.card_positions p WHERE p.id=bill.card_id AND p.owner_id=NEW.owner_id;
 IF position-NEW.amount_cents < -9223372036854775807 THEN RAISE EXCEPTION 'credit balance overflow' USING ERRCODE='ZB006'; END IF;
 IF NEW.amount_cents>remaining THEN RAISE EXCEPTION 'payment exceeds invoice balance' USING ERRCODE='ZC004'; END IF;
 RETURN NEW;
END; $$;
REVOKE ALL ON FUNCTION banking.guard_card_payment() FROM PUBLIC;
