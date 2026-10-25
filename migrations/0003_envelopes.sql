-- Writes need to inspect the parent's budget and existing allocations internally.
-- Public HTTP reads still require budget:read in the application service.
ALTER POLICY read_own_cycles ON budgeting.cycles USING (
    owner_id = NULLIF(current_setting('zerobudget.user_id', true), '')::uuid
    AND EXISTS (SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id
        WHERE u.id=owner_id AND u.enabled AND p.permission IN ('budget:read','budget:write'))
);
ALTER TABLE budgeting.cycles ADD CONSTRAINT cycles_id_owner_key UNIQUE (id, owner_id);

CREATE TABLE budgeting.envelopes (
    id uuid PRIMARY KEY,
    cycle_id uuid NOT NULL,
    owner_id uuid NOT NULL,
    name text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100 AND name=btrim(name) AND name !~ '[[:cntrl:]]'),
    allocated_cents bigint NOT NULL CHECK (allocated_cents>=0),
    FOREIGN KEY (cycle_id,owner_id) REFERENCES budgeting.cycles(id,owner_id),
    CONSTRAINT envelopes_cycle_name_key UNIQUE (cycle_id,name)
);
CREATE INDEX envelopes_owner_cycle_idx ON budgeting.envelopes(owner_id,cycle_id,id);
ALTER TABLE budgeting.envelopes ENABLE ROW LEVEL SECURITY;
ALTER TABLE budgeting.envelopes FORCE ROW LEVEL SECURITY;
CREATE POLICY read_own_envelopes ON budgeting.envelopes FOR SELECT USING (
    owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid
    AND EXISTS (SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id
        WHERE u.id=owner_id AND u.enabled AND p.permission IN ('budget:read','budget:write'))
);
CREATE POLICY insert_own_envelopes ON budgeting.envelopes FOR INSERT WITH CHECK (
    owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid
    AND EXISTS (SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id
        WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write')
);
CREATE POLICY update_own_envelopes ON budgeting.envelopes FOR UPDATE USING (
    owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid
    AND EXISTS (SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id
        WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write')
) WITH CHECK (
    owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid
    AND EXISTS (SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id
        WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write')
);
CREATE POLICY delete_own_envelopes ON budgeting.envelopes FOR DELETE USING (
    owner_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid
    AND EXISTS (SELECT 1 FROM auth.users u JOIN auth.permissions p ON p.user_id=u.id
        WHERE u.id=owner_id AND u.enabled AND p.permission='budget:write')
);

-- Defense in depth for writes issued without the application adapter. The trigger
-- is SECURITY INVOKER: it cannot bypass RLS or grant additional rights.
CREATE FUNCTION budgeting.guard_envelope_allocation() RETURNS trigger
LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE
    parent uuid;
    parent_income bigint;
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
    SELECT income_cents INTO parent_income FROM budgeting.cycles WHERE id=parent AND status='planned';
    IF NOT FOUND THEN RAISE EXCEPTION 'cycle unavailable' USING ERRCODE='42501'; END IF;
    IF TG_OP='DELETE' THEN RETURN OLD; END IF;
    -- Each query in this volatile function sees commits after acquiring the lock.
    SELECT COALESCE(sum(allocated_cents),0) INTO current_total FROM budgeting.envelopes
        WHERE cycle_id=parent AND id<>NEW.id;
    IF current_total+NEW.allocated_cents>parent_income THEN
        RAISE EXCEPTION 'allocation exceeds income' USING ERRCODE='P0001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE ALL ON FUNCTION budgeting.guard_envelope_allocation() FROM PUBLIC;
CREATE TRIGGER guard_envelope_allocation BEFORE INSERT OR UPDATE OR DELETE ON budgeting.envelopes
    FOR EACH ROW EXECUTE FUNCTION budgeting.guard_envelope_allocation();
