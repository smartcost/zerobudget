ALTER TABLE budgeting.envelopes ADD CONSTRAINT envelopes_id_cycle_owner_key UNIQUE(id,cycle_id,owner_id);
CREATE TABLE budgeting.expenses (
    id uuid PRIMARY KEY,
    cycle_id uuid NOT NULL,
    envelope_id uuid NOT NULL,
    owner_id uuid NOT NULL,
    description text NOT NULL CHECK(char_length(description) BETWEEN 1 AND 200 AND description=btrim(description) AND description !~ '[[:cntrl:]]'),
    occurred_on date NOT NULL CHECK(occurred_on BETWEEN DATE '0001-01-01' AND DATE '9999-12-31'),
    amount_cents bigint NOT NULL CHECK(amount_cents>0),
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY(envelope_id,cycle_id,owner_id) REFERENCES budgeting.envelopes(id,cycle_id,owner_id),
    UNIQUE(id,envelope_id,cycle_id,owner_id)
);
CREATE INDEX expenses_owner_envelope_date_idx ON budgeting.expenses(owner_id,cycle_id,envelope_id,occurred_on DESC,id DESC);
CREATE TABLE budgeting.expense_reversals (
    expense_id uuid PRIMARY KEY,
    cycle_id uuid NOT NULL,
    envelope_id uuid NOT NULL,
    owner_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY(expense_id,envelope_id,cycle_id,owner_id) REFERENCES budgeting.expenses(id,envelope_id,cycle_id,owner_id)
);
ALTER TABLE budgeting.expenses ENABLE ROW LEVEL SECURITY;
ALTER TABLE budgeting.expenses FORCE ROW LEVEL SECURITY;
ALTER TABLE budgeting.expense_reversals ENABLE ROW LEVEL SECURITY;
ALTER TABLE budgeting.expense_reversals FORCE ROW LEVEL SECURITY;
CREATE POLICY read_own_expenses ON budgeting.expenses FOR SELECT USING (
    owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid
    AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id
        WHERE u.id=owner_id AND u.enabled AND p.permission IN('budget:read','budget:write'))
);
CREATE POLICY insert_own_expenses ON budgeting.expenses FOR INSERT WITH CHECK (
    owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid
    AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id
        WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write')
);
CREATE POLICY read_own_reversals ON budgeting.expense_reversals FOR SELECT USING (
    owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid
    AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id
        WHERE u.id=owner_id AND u.enabled AND p.permission IN('budget:read','budget:write'))
);
CREATE POLICY insert_own_reversals ON budgeting.expense_reversals FOR INSERT WITH CHECK (
    owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid
    AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id
        WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write')
);
-- The same lock coordinates spending, reversals, allocation and closure. These
-- invoker functions never bypass RLS, even for SQL issued without an adapter.
CREATE FUNCTION budgeting.guard_expense_insert() RETURNS trigger
LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE parent_status text; period_start date; period_end date; allocation bigint; spent numeric;
BEGIN
    IF current_setting('transaction_isolation')<>'read committed' THEN
        RAISE EXCEPTION 'expenses require read committed' USING ERRCODE='25000';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('zerobudget.envelopes:'||NEW.cycle_id::text,0));
    SELECT c.status,c.period_start,c.period_end,e.allocated_cents INTO parent_status,period_start,period_end,allocation
        FROM budgeting.cycles c JOIN budgeting.envelopes e ON e.cycle_id=c.id AND e.owner_id=c.owner_id
        WHERE c.id=NEW.cycle_id AND c.owner_id=NEW.owner_id AND e.id=NEW.envelope_id;
    IF NOT FOUND THEN RAISE EXCEPTION 'budget resource unavailable' USING ERRCODE='42501'; END IF;
    IF parent_status<>'active' THEN RAISE EXCEPTION 'cycle is not active' USING ERRCODE='ZB002'; END IF;
    IF TG_TABLE_NAME='expenses' THEN
        IF NEW.occurred_on<period_start OR NEW.occurred_on>=period_end THEN
            RAISE EXCEPTION 'expense date outside cycle' USING ERRCODE='ZB004';
        END IF;
        SELECT COALESCE(sum(x.amount_cents),0) INTO spent FROM budgeting.expenses x
            WHERE x.envelope_id=NEW.envelope_id AND x.cycle_id=NEW.cycle_id AND x.owner_id=NEW.owner_id
            AND NOT EXISTS(SELECT 1 FROM budgeting.expense_reversals r WHERE r.expense_id=x.id);
        IF spent+NEW.amount_cents>allocation THEN
            RAISE EXCEPTION 'insufficient envelope balance' USING ERRCODE='ZB003';
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION budgeting.guard_expense_insert() FROM PUBLIC;
CREATE TRIGGER guard_expense_insert BEFORE INSERT ON budgeting.expenses
    FOR EACH ROW EXECUTE FUNCTION budgeting.guard_expense_insert();
CREATE TRIGGER guard_reversal_insert BEFORE INSERT ON budgeting.expense_reversals
    FOR EACH ROW EXECUTE FUNCTION budgeting.guard_expense_insert();
-- Reject edits even if an operator accidentally broadens the runtime grants.
CREATE FUNCTION budgeting.guard_immutable_expense() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN RAISE EXCEPTION 'expense ledger is immutable' USING ERRCODE='42501'; END;
$$;
REVOKE ALL ON FUNCTION budgeting.guard_immutable_expense() FROM PUBLIC;
CREATE TRIGGER immutable_expense BEFORE UPDATE OR DELETE ON budgeting.expenses
    FOR EACH ROW EXECUTE FUNCTION budgeting.guard_immutable_expense();
CREATE TRIGGER immutable_reversal BEFORE UPDATE OR DELETE ON budgeting.expense_reversals
    FOR EACH ROW EXECUTE FUNCTION budgeting.guard_immutable_expense();
