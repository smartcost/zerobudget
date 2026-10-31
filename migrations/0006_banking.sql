CREATE SCHEMA banking;
REVOKE ALL ON SCHEMA banking FROM PUBLIC;
CREATE TABLE banking.accounts (
 id uuid PRIMARY KEY, owner_id uuid NOT NULL REFERENCES auth.users(id),
 name text NOT NULL CHECK(char_length(name) BETWEEN 1 AND 100 AND name=btrim(name) AND name !~ '[[:cntrl:]]'),
 kind text NOT NULL CHECK(kind IN('bank','cash')),
 opened_on date NOT NULL CHECK(opened_on BETWEEN DATE '0001-01-01' AND DATE '9999-12-31'),
 opening_balance_cents bigint NOT NULL CHECK(opening_balance_cents>=0),
 created_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(id,owner_id),CONSTRAINT accounts_owner_name_key UNIQUE(owner_id,name)
);
ALTER TABLE budgeting.expenses ADD COLUMN account_id uuid;
ALTER TABLE budgeting.expenses ADD CONSTRAINT expenses_account_owner_fk FOREIGN KEY(account_id,owner_id) REFERENCES banking.accounts(id,owner_id);
-- Preserve old unlinked rows, while requiring a payment account on new writes.
ALTER TABLE budgeting.expenses ADD CONSTRAINT expenses_account_required CHECK(account_id IS NOT NULL) NOT VALID;
ALTER TABLE budgeting.expenses ADD CONSTRAINT expenses_id_owner_key UNIQUE(id,owner_id);
CREATE TABLE banking.movements (
 id uuid PRIMARY KEY,owner_id uuid NOT NULL REFERENCES auth.users(id),
 kind text NOT NULL CHECK(kind IN('receipt','transfer','expense','expense_reversal')),
 from_account_id uuid,to_account_id uuid,
 description text NOT NULL CHECK(char_length(description) BETWEEN 1 AND 200 AND description=btrim(description) AND description !~ '[[:cntrl:]]'),
 occurred_on date NOT NULL CHECK(occurred_on BETWEEN DATE '0001-01-01' AND DATE '9999-12-31'),
 amount_cents bigint NOT NULL CHECK(amount_cents>0),expense_id uuid,
 created_at timestamptz NOT NULL DEFAULT now(),
 FOREIGN KEY(from_account_id,owner_id) REFERENCES banking.accounts(id,owner_id),
 FOREIGN KEY(to_account_id,owner_id) REFERENCES banking.accounts(id,owner_id),
 FOREIGN KEY(expense_id,owner_id) REFERENCES budgeting.expenses(id,owner_id),
 UNIQUE(kind,expense_id),
 CHECK((kind='receipt' AND from_account_id IS NULL AND to_account_id IS NOT NULL AND expense_id IS NULL)
 OR (kind='transfer' AND from_account_id IS NOT NULL AND to_account_id IS NOT NULL AND from_account_id<>to_account_id AND expense_id IS NULL)
 OR (kind='expense' AND from_account_id IS NOT NULL AND to_account_id IS NULL AND expense_id IS NOT NULL)
 OR (kind='expense_reversal' AND from_account_id IS NULL AND to_account_id IS NOT NULL AND expense_id IS NOT NULL))
);
CREATE INDEX movements_from_idx ON banking.movements(owner_id,from_account_id,occurred_on DESC,id DESC);
CREATE INDEX movements_to_idx ON banking.movements(owner_id,to_account_id,occurred_on DESC,id DESC);
ALTER TABLE banking.accounts ENABLE ROW LEVEL SECURITY;
ALTER TABLE banking.accounts FORCE ROW LEVEL SECURITY;
ALTER TABLE banking.movements ENABLE ROW LEVEL SECURITY;
ALTER TABLE banking.movements FORCE ROW LEVEL SECURITY;
CREATE POLICY read_accounts ON banking.accounts FOR SELECT USING(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission IN('budget:read','budget:write')));
CREATE POLICY insert_accounts ON banking.accounts FOR INSERT WITH CHECK(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write'));
CREATE POLICY read_movements ON banking.movements FOR SELECT USING(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission IN('budget:read','budget:write')));
CREATE POLICY insert_movements ON banking.movements FOR INSERT WITH CHECK(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write'));
CREATE FUNCTION banking.immutable() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN RAISE EXCEPTION 'banking history is immutable' USING ERRCODE='42501'; END; $$;
REVOKE ALL ON FUNCTION banking.immutable() FROM PUBLIC;
CREATE TRIGGER immutable_account BEFORE UPDATE OR DELETE ON banking.accounts FOR EACH ROW EXECUTE FUNCTION banking.immutable();
CREATE TRIGGER immutable_movement BEFORE UPDATE OR DELETE ON banking.movements FOR EACH ROW EXECUTE FUNCTION banking.immutable();
CREATE FUNCTION banking.guard_movement() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE a record; balance numeric; next_balance numeric; payment record;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' THEN RAISE EXCEPTION 'banking writes require read committed' USING ERRCODE='25000'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('zerobudget.banking:'||NEW.owner_id::text,0));
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
CREATE TRIGGER guard_movement BEFORE INSERT ON banking.movements FOR EACH ROW EXECUTE FUNCTION banking.guard_movement();
-- Expense triggers already hold the cycle lock. Always acquire the banking lock
-- after it; cash-only operations never acquire cycle locks.
CREATE FUNCTION banking.record_payment() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE x record;
BEGIN
 IF TG_TABLE_NAME='expenses' THEN
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
CREATE TRIGGER record_expense_payment AFTER INSERT ON budgeting.expenses FOR EACH ROW EXECUTE FUNCTION banking.record_payment();
CREATE TRIGGER record_expense_refund AFTER INSERT ON budgeting.expense_reversals FOR EACH ROW EXECUTE FUNCTION banking.record_payment();
