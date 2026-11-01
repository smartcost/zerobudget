CREATE TABLE budgeting.reallocations (
 id uuid PRIMARY KEY,owner_id uuid NOT NULL,cycle_id uuid NOT NULL,
 from_envelope_id uuid NOT NULL,to_envelope_id uuid NOT NULL,
 amount_cents bigint NOT NULL CHECK(amount_cents>0),created_at timestamptz NOT NULL DEFAULT now(),
 CHECK(from_envelope_id<>to_envelope_id),
 FOREIGN KEY(from_envelope_id,cycle_id,owner_id) REFERENCES budgeting.envelopes(id,cycle_id,owner_id),
 FOREIGN KEY(to_envelope_id,cycle_id,owner_id) REFERENCES budgeting.envelopes(id,cycle_id,owner_id)
);
CREATE INDEX reallocations_cycle_idx ON budgeting.reallocations(owner_id,cycle_id);
ALTER TABLE budgeting.reallocations ENABLE ROW LEVEL SECURITY;
ALTER TABLE budgeting.reallocations FORCE ROW LEVEL SECURITY;
CREATE POLICY reallocations_read ON budgeting.reallocations FOR SELECT USING(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission IN('budget:read','budget:write')));
CREATE POLICY reallocations_insert ON budgeting.reallocations FOR INSERT WITH CHECK(owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid AND EXISTS(SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write'));
CREATE TRIGGER immutable_reallocation BEFORE UPDATE OR DELETE ON budgeting.reallocations FOR EACH ROW EXECUTE FUNCTION banking.immutable();
CREATE VIEW budgeting.envelope_balances WITH(security_invoker=true) AS
 SELECT e.id,e.owner_id,e.cycle_id,e.name,
 (e.allocated_cents+COALESCE((SELECT sum(CASE WHEN r.to_envelope_id=e.id THEN r.amount_cents::numeric ELSE -r.amount_cents::numeric END) FROM budgeting.reallocations r WHERE r.owner_id=e.owner_id AND r.cycle_id=e.cycle_id AND (r.from_envelope_id=e.id OR r.to_envelope_id=e.id)),0))::bigint AS allocated_cents,
 (SELECT COALESCE(sum(x.amount_cents),0)::bigint FROM budgeting.expenses x WHERE x.owner_id=e.owner_id AND x.cycle_id=e.cycle_id AND x.envelope_id=e.id AND NOT EXISTS(SELECT 1 FROM budgeting.expense_reversals z WHERE z.expense_id=x.id)) AS spent_cents
 FROM budgeting.envelopes e;
CREATE FUNCTION budgeting.guard_reallocation() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE source record;target record;cycle_status text;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' THEN RAISE EXCEPTION 'planning writes require read committed' USING ERRCODE='25000'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('zerobudget.envelopes:'||NEW.cycle_id::text,0));
 SELECT status INTO cycle_status FROM budgeting.cycles WHERE id=NEW.cycle_id AND owner_id=NEW.owner_id;
 IF NOT FOUND THEN RAISE EXCEPTION 'cycle unavailable' USING ERRCODE='42501'; END IF;
 IF cycle_status<>'active' THEN RAISE EXCEPTION 'cycle must be active' USING ERRCODE='ZB002'; END IF;
 SELECT allocated_cents,spent_cents INTO source FROM budgeting.envelope_balances WHERE id=NEW.from_envelope_id AND cycle_id=NEW.cycle_id AND owner_id=NEW.owner_id;
 IF NOT FOUND THEN RAISE EXCEPTION 'source unavailable' USING ERRCODE='42501'; END IF;
 SELECT allocated_cents INTO target FROM budgeting.envelope_balances WHERE id=NEW.to_envelope_id AND cycle_id=NEW.cycle_id AND owner_id=NEW.owner_id;
 IF NOT FOUND THEN RAISE EXCEPTION 'target unavailable' USING ERRCODE='42501'; END IF;
 IF NEW.amount_cents>source.allocated_cents-source.spent_cents THEN RAISE EXCEPTION 'insufficient unspent allocation' USING ERRCODE='ZB003'; END IF;
 IF target.allocated_cents::numeric+NEW.amount_cents>9223372036854775807 THEN RAISE EXCEPTION 'allocation overflow' USING ERRCODE='ZB006'; END IF;
 RETURN NEW;
END; $$;
REVOKE ALL ON FUNCTION budgeting.guard_reallocation() FROM PUBLIC;
CREATE TRIGGER guard_reallocation BEFORE INSERT ON budgeting.reallocations FOR EACH ROW EXECUTE FUNCTION budgeting.guard_reallocation();
ALTER TABLE budgeting.cycles ADD COLUMN carried_cents bigint NOT NULL DEFAULT 0 CHECK(carried_cents>=0 AND carried_cents<=income_cents);
ALTER TABLE budgeting.cycles ADD COLUMN source_cycle_id uuid;
ALTER TABLE budgeting.cycles ADD COLUMN copy_request_id uuid;
ALTER TABLE budgeting.cycles ADD COLUMN copy_mode text CHECK(copy_mode IN('empty','copy','carry_remaining'));
ALTER TABLE budgeting.cycles ADD CONSTRAINT cycles_source_owner_fk FOREIGN KEY(source_cycle_id,owner_id) REFERENCES budgeting.cycles(id,owner_id);
ALTER TABLE budgeting.cycles ADD CONSTRAINT cycles_one_successor UNIQUE(source_cycle_id);
ALTER TABLE budgeting.cycles ADD CONSTRAINT cycles_copy_request_key UNIQUE(owner_id,copy_request_id);
ALTER TABLE budgeting.cycles ADD CONSTRAINT cycles_source_metadata CHECK((source_cycle_id IS NULL AND copy_request_id IS NULL AND copy_mode IS NULL AND carried_cents=0) OR(source_cycle_id IS NOT NULL AND source_cycle_id<>id AND copy_request_id IS NOT NULL AND copy_mode IS NOT NULL));
CREATE FUNCTION budgeting.guard_cycle_copy() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE source record;remaining numeric;
BEGIN
 IF NEW.source_cycle_id IS NULL THEN RETURN NEW; END IF;
 IF current_setting('transaction_isolation')<>'read committed' THEN RAISE EXCEPTION 'planning writes require read committed' USING ERRCODE='25000'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('zerobudget.envelopes:'||NEW.source_cycle_id::text,0));
 SELECT status,period_end INTO source FROM budgeting.cycles WHERE id=NEW.source_cycle_id AND owner_id=NEW.owner_id;
 IF NOT FOUND THEN RAISE EXCEPTION 'source unavailable' USING ERRCODE='42501'; END IF;
 IF source.status<>'closed' THEN RAISE EXCEPTION 'source must be closed' USING ERRCODE='ZB002'; END IF;
 IF NEW.period_start<source.period_end THEN RAISE EXCEPTION 'successor overlaps source' USING ERRCODE='ZB004'; END IF;
 SELECT COALESCE(sum(allocated_cents-spent_cents),0) INTO remaining FROM budgeting.envelope_balances WHERE cycle_id=NEW.source_cycle_id AND owner_id=NEW.owner_id;
 IF NEW.carried_cents<>(CASE WHEN NEW.copy_mode='carry_remaining' THEN remaining ELSE 0 END) THEN RAISE EXCEPTION 'invalid carry amount' USING ERRCODE='ZB004'; END IF;
 RETURN NEW;
END; $$;
REVOKE ALL ON FUNCTION budgeting.guard_cycle_copy() FROM PUBLIC;
CREATE TRIGGER guard_cycle_copy BEFORE INSERT ON budgeting.cycles FOR EACH ROW EXECUTE FUNCTION budgeting.guard_cycle_copy();

CREATE OR REPLACE FUNCTION budgeting.guard_expense_insert() RETURNS trigger
LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE parent_status text; period_start date; period_end date; allocation bigint; spent numeric;
BEGIN
    IF current_setting('transaction_isolation')<>'read committed' THEN
        RAISE EXCEPTION 'expenses require read committed' USING ERRCODE='25000';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('zerobudget.envelopes:'||NEW.cycle_id::text,0));
    SELECT c.status,c.period_start,c.period_end,e.allocated_cents INTO parent_status,period_start,period_end,allocation
        FROM budgeting.cycles c JOIN budgeting.envelope_balances e ON e.cycle_id=c.id AND e.owner_id=c.owner_id
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

CREATE OR REPLACE FUNCTION budgeting.guard_cycle_transition() RETURNS trigger
LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE total numeric;
BEGIN
    IF current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'cycle transitions require read committed' USING ERRCODE='25000';
    END IF;
    IF (NEW.id,NEW.owner_id,NEW.period_start,NEW.period_end,NEW.income_cents,NEW.currency,NEW.created_at,NEW.carried_cents,NEW.source_cycle_id,NEW.copy_request_id,NEW.copy_mode)
        IS DISTINCT FROM (OLD.id,OLD.owner_id,OLD.period_start,OLD.period_end,OLD.income_cents,OLD.currency,OLD.created_at,OLD.carried_cents,OLD.source_cycle_id,OLD.copy_request_id,OLD.copy_mode) THEN
        RAISE EXCEPTION 'immutable cycle fields' USING ERRCODE='23514';
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('zerobudget.envelopes:'||OLD.id::text,0));
    IF NEW.status=OLD.status THEN RETURN NEW; END IF;
    IF OLD.status='planned' AND NEW.status='active' THEN
        SELECT COALESCE(sum(allocated_cents),0) INTO total FROM budgeting.envelopes
            WHERE cycle_id=OLD.id AND owner_id=OLD.owner_id;
        IF total<>OLD.income_cents THEN
            RAISE EXCEPTION 'cycle income is not fully allocated' USING ERRCODE='ZB001';
        END IF;
    ELSIF NOT (OLD.status='active' AND NEW.status='closed') THEN
        RAISE EXCEPTION 'invalid cycle transition' USING ERRCODE='ZB002';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION budgeting.guard_cycle_transition() FROM PUBLIC;
