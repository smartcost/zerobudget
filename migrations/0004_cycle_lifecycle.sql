ALTER TABLE budgeting.cycles DROP CONSTRAINT cycles_status_check;
ALTER TABLE budgeting.cycles ADD CONSTRAINT cycles_status_check CHECK (status IN ('planned','active','closed'));

CREATE POLICY update_own_cycles ON budgeting.cycles FOR UPDATE USING (
    owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid
    AND EXISTS (SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id
        WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write')
) WITH CHECK (
    owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid
    AND EXISTS (SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id
        WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write')
);

-- Invoker functions preserve RLS. The runtime role can UPDATE only status.
CREATE FUNCTION budgeting.guard_cycle_transition() RETURNS trigger
LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE total numeric;
BEGIN
    IF current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'cycle transitions require read committed' USING ERRCODE='25000';
    END IF;
    IF (NEW.id,NEW.owner_id,NEW.period_start,NEW.period_end,NEW.income_cents,NEW.currency,NEW.created_at)
        IS DISTINCT FROM (OLD.id,OLD.owner_id,OLD.period_start,OLD.period_end,OLD.income_cents,OLD.currency,OLD.created_at) THEN
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
CREATE TRIGGER guard_cycle_transition BEFORE UPDATE ON budgeting.cycles
    FOR EACH ROW EXECUTE FUNCTION budgeting.guard_cycle_transition();

-- Keep the existing allocation lock and guard, now distinguishing a missing
-- parent from a frozen plan. Reads remain available in every lifecycle state.
CREATE OR REPLACE FUNCTION budgeting.guard_envelope_allocation() RETURNS trigger
LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE
    parent uuid;
    parent_income bigint;
    parent_status text;
    current_total numeric;
BEGIN
    IF current_setting('transaction_isolation') <> 'read committed' THEN
        RAISE EXCEPTION 'envelope writes require read committed' USING ERRCODE='25000';
    END IF;
    IF TG_OP='UPDATE' AND (NEW.id,NEW.cycle_id,NEW.owner_id) IS DISTINCT FROM (OLD.id,OLD.cycle_id,OLD.owner_id) THEN
        RAISE EXCEPTION 'immutable envelope ownership' USING ERRCODE='23514';
    END IF;
    IF TG_OP='DELETE' THEN parent:=OLD.cycle_id; ELSE parent:=NEW.cycle_id; END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('zerobudget.envelopes:'||parent::text,0));
    SELECT income_cents,status INTO parent_income,parent_status FROM budgeting.cycles WHERE id=parent;
    IF NOT FOUND THEN RAISE EXCEPTION 'cycle unavailable' USING ERRCODE='42501'; END IF;
    IF parent_status<>'planned' THEN
        RAISE EXCEPTION 'cycle plan is frozen' USING ERRCODE='ZB002';
    END IF;
    IF TG_OP='DELETE' THEN RETURN OLD; END IF;
    SELECT COALESCE(sum(allocated_cents),0) INTO current_total FROM budgeting.envelopes
        WHERE cycle_id=parent AND id<>NEW.id;
    IF current_total+NEW.allocated_cents>parent_income THEN
        RAISE EXCEPTION 'allocation exceeds income' USING ERRCODE='P0001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION budgeting.guard_envelope_allocation() FROM PUBLIC;
