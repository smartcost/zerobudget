CREATE TABLE banking.credit_cards (
 id uuid PRIMARY KEY,owner_id uuid NOT NULL REFERENCES auth.users(id),
 name text NOT NULL CHECK(char_length(name) BETWEEN 1 AND 100 AND name=btrim(name) AND name !~ '[[:cntrl:]]'),
 opened_on date NOT NULL CHECK(opened_on BETWEEN DATE '0001-01-01' AND DATE '9999-12-31'),
 credit_limit_cents bigint NOT NULL CHECK(credit_limit_cents>0),created_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(id,owner_id),CONSTRAINT cards_owner_name_key UNIQUE(owner_id,name)
);
CREATE TABLE banking.card_invoices (
 id uuid PRIMARY KEY,owner_id uuid NOT NULL,card_id uuid NOT NULL,
 period_start date NOT NULL,period_end date NOT NULL,due_on date NOT NULL,
 status text NOT NULL DEFAULT 'open' CHECK(status IN('open','closed')),
 created_at timestamptz NOT NULL DEFAULT now(),
 CHECK(period_start>=DATE '0001-01-01' AND period_end>period_start AND due_on>=period_end AND due_on<=DATE '9999-12-31'),
 FOREIGN KEY(card_id,owner_id) REFERENCES banking.credit_cards(id,owner_id),UNIQUE(id,owner_id)
);
CREATE INDEX invoices_card_period_idx ON banking.card_invoices(owner_id,card_id,period_start DESC,id DESC);
ALTER TABLE budgeting.expenses ADD COLUMN invoice_id uuid;
ALTER TABLE budgeting.expenses ADD CONSTRAINT expenses_invoice_owner_fk FOREIGN KEY(invoice_id,owner_id) REFERENCES banking.card_invoices(id,owner_id);
ALTER TABLE budgeting.expenses DROP CONSTRAINT expenses_account_required;
ALTER TABLE budgeting.expenses ADD CONSTRAINT expenses_funding_required CHECK(num_nonnulls(account_id,invoice_id)=1) NOT VALID;
CREATE INDEX expenses_invoice_idx ON budgeting.expenses(owner_id,invoice_id,id);
CREATE TABLE banking.card_payments (
 id uuid PRIMARY KEY,owner_id uuid NOT NULL,invoice_id uuid NOT NULL,account_id uuid NOT NULL,
 description text NOT NULL CHECK(char_length(description) BETWEEN 1 AND 200 AND description=btrim(description) AND description !~ '[[:cntrl:]]'),
 occurred_on date NOT NULL CHECK(occurred_on BETWEEN DATE '0001-01-01' AND DATE '9999-12-31'),
 amount_cents bigint NOT NULL CHECK(amount_cents>0),created_at timestamptz NOT NULL DEFAULT now(),
 FOREIGN KEY(invoice_id,owner_id) REFERENCES banking.card_invoices(id,owner_id),
 FOREIGN KEY(account_id,owner_id) REFERENCES banking.accounts(id,owner_id),UNIQUE(id,owner_id)
);
CREATE INDEX payments_invoice_idx ON banking.card_payments(owner_id,invoice_id,occurred_on DESC,id DESC);
ALTER TABLE banking.movements ADD COLUMN card_payment_id uuid;
ALTER TABLE banking.movements ADD CONSTRAINT movements_card_payment_fk FOREIGN KEY(card_payment_id,owner_id) REFERENCES banking.card_payments(id,owner_id);
ALTER TABLE banking.movements ADD CONSTRAINT movements_card_payment_key UNIQUE(card_payment_id);
ALTER TABLE banking.movements DROP CONSTRAINT movements_kind_check;
ALTER TABLE banking.movements DROP CONSTRAINT movements_check;
ALTER TABLE banking.movements ADD CONSTRAINT movements_kind_check CHECK(kind IN('receipt','transfer','expense','expense_reversal','card_payment'));
ALTER TABLE banking.movements ADD CONSTRAINT movements_check CHECK(
 ((kind='receipt' AND from_account_id IS NULL AND to_account_id IS NOT NULL AND expense_id IS NULL)
 OR(kind='transfer' AND from_account_id IS NOT NULL AND to_account_id IS NOT NULL AND from_account_id<>to_account_id AND expense_id IS NULL)
 OR(kind='expense' AND from_account_id IS NOT NULL AND to_account_id IS NULL AND expense_id IS NOT NULL)
 OR(kind='expense_reversal' AND from_account_id IS NULL AND to_account_id IS NOT NULL AND expense_id IS NOT NULL)) AND card_payment_id IS NULL
 OR(kind='card_payment' AND from_account_id IS NOT NULL AND to_account_id IS NULL AND expense_id IS NULL AND card_payment_id IS NOT NULL)
);

ALTER TABLE banking.credit_cards ENABLE ROW LEVEL SECURITY;
ALTER TABLE banking.credit_cards FORCE ROW LEVEL SECURITY;
CREATE POLICY credit_cards_select ON banking.credit_cards FOR SELECT USING(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission IN('budget:read','budget:write')));
CREATE POLICY credit_cards_insert ON banking.credit_cards FOR INSERT WITH CHECK(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write'));

ALTER TABLE banking.card_invoices ENABLE ROW LEVEL SECURITY;
ALTER TABLE banking.card_invoices FORCE ROW LEVEL SECURITY;
CREATE POLICY card_invoices_select ON banking.card_invoices FOR SELECT USING(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission IN('budget:read','budget:write')));
CREATE POLICY card_invoices_insert ON banking.card_invoices FOR INSERT WITH CHECK(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write'));

ALTER TABLE banking.card_payments ENABLE ROW LEVEL SECURITY;
ALTER TABLE banking.card_payments FORCE ROW LEVEL SECURITY;
CREATE POLICY card_payments_select ON banking.card_payments FOR SELECT USING(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission IN('budget:read','budget:write')));
CREATE POLICY card_payments_insert ON banking.card_payments FOR INSERT WITH CHECK(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write'));

CREATE POLICY invoice_update ON banking.card_invoices FOR UPDATE
 USING(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write'))
 WITH CHECK(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write'));
CREATE TRIGGER immutable_card BEFORE UPDATE OR DELETE ON banking.credit_cards FOR EACH ROW EXECUTE FUNCTION banking.immutable();
CREATE TRIGGER immutable_card_payment BEFORE UPDATE OR DELETE ON banking.card_payments FOR EACH ROW EXECUTE FUNCTION banking.immutable();
CREATE FUNCTION banking.guard_invoice() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' THEN RAISE EXCEPTION 'credit writes require read committed' USING ERRCODE='25000'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('zerobudget.banking:'||NEW.owner_id::text,0));
 IF TG_OP='UPDATE' THEN
  IF (NEW.id,NEW.owner_id,NEW.card_id,NEW.period_start,NEW.period_end,NEW.due_on,NEW.created_at) IS DISTINCT FROM (OLD.id,OLD.owner_id,OLD.card_id,OLD.period_start,OLD.period_end,OLD.due_on,OLD.created_at) THEN RAISE EXCEPTION 'immutable invoice fields' USING ERRCODE='42501'; END IF;
  IF NEW.status<>OLD.status AND NOT(OLD.status='open' AND NEW.status='closed') THEN RAISE EXCEPTION 'invalid invoice transition' USING ERRCODE='ZC002'; END IF;
 ELSE
  IF NEW.status<>'open' THEN RAISE EXCEPTION 'new invoices must be open' USING ERRCODE='ZC002'; END IF;
  IF EXISTS(SELECT 1 FROM banking.card_invoices i WHERE i.owner_id=NEW.owner_id AND i.card_id=NEW.card_id AND i.period_start<NEW.period_end AND i.period_end>NEW.period_start) THEN RAISE EXCEPTION 'overlapping invoice periods' USING ERRCODE='ZC003'; END IF;
 END IF;
 RETURN NEW;
END; $$;
REVOKE ALL ON FUNCTION banking.guard_invoice() FROM PUBLIC;
CREATE TRIGGER guard_invoice BEFORE INSERT OR UPDATE ON banking.card_invoices FOR EACH ROW EXECUTE FUNCTION banking.guard_invoice();
CREATE FUNCTION banking.guard_credit_expense() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE invoice uuid;bill record;debt numeric;
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
  SELECT COALESCE((SELECT sum(x.amount_cents::numeric) FROM budgeting.expenses x JOIN banking.card_invoices i ON i.id=x.invoice_id AND i.owner_id=x.owner_id WHERE i.card_id=bill.card_id AND x.owner_id=NEW.owner_id AND NOT EXISTS(SELECT 1 FROM budgeting.expense_reversals r WHERE r.expense_id=x.id)),0)-COALESCE((SELECT sum(p.amount_cents::numeric) FROM banking.card_payments p JOIN banking.card_invoices i ON i.id=p.invoice_id AND i.owner_id=p.owner_id WHERE i.card_id=bill.card_id AND p.owner_id=NEW.owner_id),0) INTO debt;
  IF debt+NEW.amount_cents>bill.credit_limit_cents THEN RAISE EXCEPTION 'credit limit exceeded' USING ERRCODE='ZC001'; END IF;
 END IF;
 RETURN NEW;
END; $$;
REVOKE ALL ON FUNCTION banking.guard_credit_expense() FROM PUBLIC;
CREATE TRIGGER validate_credit_purchase BEFORE INSERT ON budgeting.expenses FOR EACH ROW EXECUTE FUNCTION banking.guard_credit_expense();
CREATE TRIGGER validate_credit_reversal BEFORE INSERT ON budgeting.expense_reversals FOR EACH ROW EXECUTE FUNCTION banking.guard_credit_expense();
CREATE FUNCTION banking.guard_card_payment() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE bill record;remaining numeric;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' THEN RAISE EXCEPTION 'credit writes require read committed' USING ERRCODE='25000'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('zerobudget.banking:'||NEW.owner_id::text,0));
 SELECT status,period_end INTO bill FROM banking.card_invoices WHERE id=NEW.invoice_id AND owner_id=NEW.owner_id;
 IF NOT FOUND THEN RAISE EXCEPTION 'invoice unavailable' USING ERRCODE='42501'; END IF;
 IF bill.status<>'closed' THEN RAISE EXCEPTION 'invoice must be closed' USING ERRCODE='ZC002'; END IF;
 IF NEW.occurred_on<bill.period_end THEN RAISE EXCEPTION 'payment precedes invoice period end' USING ERRCODE='ZB004'; END IF;
 SELECT COALESCE((SELECT sum(x.amount_cents::numeric) FROM budgeting.expenses x WHERE x.invoice_id=NEW.invoice_id AND x.owner_id=NEW.owner_id AND NOT EXISTS(SELECT 1 FROM budgeting.expense_reversals r WHERE r.expense_id=x.id)),0)-COALESCE((SELECT sum(amount_cents::numeric) FROM banking.card_payments WHERE invoice_id=NEW.invoice_id AND owner_id=NEW.owner_id),0) INTO remaining;
 IF NEW.amount_cents>remaining THEN RAISE EXCEPTION 'payment exceeds invoice balance' USING ERRCODE='ZC004'; END IF;
 RETURN NEW;
END; $$;
REVOKE ALL ON FUNCTION banking.guard_card_payment() FROM PUBLIC;
CREATE TRIGGER guard_card_payment BEFORE INSERT ON banking.card_payments FOR EACH ROW EXECUTE FUNCTION banking.guard_card_payment();
CREATE FUNCTION banking.record_card_payment() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
BEGIN
 INSERT INTO banking.movements(id,owner_id,kind,from_account_id,description,occurred_on,amount_cents,card_payment_id) VALUES(gen_random_uuid(),NEW.owner_id,'card_payment',NEW.account_id,NEW.description,NEW.occurred_on,NEW.amount_cents,NEW.id);
 RETURN NEW;
END; $$;
REVOKE ALL ON FUNCTION banking.record_card_payment() FROM PUBLIC;
CREATE TRIGGER record_card_payment AFTER INSERT ON banking.card_payments FOR EACH ROW EXECUTE FUNCTION banking.record_card_payment();

CREATE OR REPLACE FUNCTION banking.guard_movement() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE a record; balance numeric; next_balance numeric; payment record;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' THEN RAISE EXCEPTION 'banking writes require read committed' USING ERRCODE='25000'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('zerobudget.banking:'||NEW.owner_id::text,0));
 IF NEW.kind='card_payment' THEN
   SELECT account_id,amount_cents,occurred_on,description INTO payment FROM banking.card_payments WHERE id=NEW.card_payment_id AND owner_id=NEW.owner_id;
   IF NOT FOUND OR payment.account_id<>NEW.from_account_id OR payment.amount_cents<>NEW.amount_cents OR payment.occurred_on<>NEW.occurred_on OR payment.description<>NEW.description THEN RAISE EXCEPTION 'invalid card settlement' USING ERRCODE='ZB004'; END IF;
 END IF;
 IF NEW.kind IN('expense','expense_reversal') THEN
   SELECT x.account_id,x.amount_cents,x.occurred_on,x.description INTO payment FROM budgeting.expenses x WHERE x.id=NEW.expense_id AND x.owner_id=NEW.owner_id;
   IF NOT FOUND OR payment.account_id IS NULL OR payment.account_id<>COALESCE(NEW.from_account_id,NEW.to_account_id) OR payment.amount_cents<>NEW.amount_cents OR payment.occurred_on<>NEW.occurred_on OR payment.description<>NEW.description THEN RAISE EXCEPTION 'invalid expense payment' USING ERRCODE='ZB004'; END IF;
   IF NEW.kind='expense_reversal' AND NOT EXISTS(SELECT 1 FROM budgeting.expense_reversals r WHERE r.expense_id=NEW.expense_id AND r.owner_id=NEW.owner_id) THEN RAISE EXCEPTION 'reversal marker missing' USING ERRCODE='ZB004'; END IF;
 END IF;
 FOR a IN SELECT id,opened_on,opening_balance_cents FROM banking.accounts WHERE owner_id=NEW.owner_id AND id IN(NEW.from_account_id,NEW.to_account_id) ORDER BY id LOOP
   IF NEW.occurred_on<a.opened_on THEN RAISE EXCEPTION 'movement before opening date' USING ERRCODE='ZB004'; END IF;
   SELECT a.opening_balance_cents+COALESCE(sum(CASE WHEN m.to_account_id=a.id THEN m.amount_cents::numeric ELSE -m.amount_cents::numeric END),0) INTO balance FROM banking.movements m WHERE m.owner_id=NEW.owner_id AND (m.from_account_id=a.id OR m.to_account_id=a.id);
   next_balance:=balance+CASE WHEN NEW.to_account_id=a.id THEN NEW.amount_cents::numeric ELSE -NEW.amount_cents::numeric END;
   IF next_balance<0 THEN RAISE EXCEPTION 'insufficient account balance' USING ERRCODE='ZB005'; END IF;
   IF next_balance>9223372036854775807 THEN RAISE EXCEPTION 'account balance overflow' USING ERRCODE='ZB006'; END IF;
 END LOOP;
 RETURN NEW;
END; $$;
REVOKE ALL ON FUNCTION banking.guard_movement() FROM PUBLIC;

CREATE OR REPLACE FUNCTION banking.record_payment() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE x record;
BEGIN
 IF TG_TABLE_NAME='expenses' THEN
   IF NEW.account_id IS NULL THEN RETURN NEW; END IF;
   INSERT INTO banking.movements(id,owner_id,kind,from_account_id,description,occurred_on,amount_cents,expense_id)
     VALUES(gen_random_uuid(),NEW.owner_id,'expense',NEW.account_id,NEW.description,NEW.occurred_on,NEW.amount_cents,NEW.id);
 ELSE
   SELECT * INTO x FROM budgeting.expenses WHERE id=NEW.expense_id AND owner_id=NEW.owner_id;
   IF x.account_id IS NOT NULL THEN
     INSERT INTO banking.movements(id,owner_id,kind,to_account_id,description,occurred_on,amount_cents,expense_id)
       VALUES(gen_random_uuid(),NEW.owner_id,'expense_reversal',x.account_id,x.description,x.occurred_on,x.amount_cents,x.id);
   END IF;
 END IF;
 RETURN NEW;
END; $$;
REVOKE ALL ON FUNCTION banking.record_payment() FROM PUBLIC;
