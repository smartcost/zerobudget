-- Explicit, one-time enrollment; runtime still cannot directly grant permissions.
CREATE TABLE auth.budget_onboardings (
 user_id uuid PRIMARY KEY REFERENCES auth.users(id),
 completed_at timestamptz NOT NULL DEFAULT now()
);
ALTER TABLE auth.budget_onboardings ENABLE ROW LEVEL SECURITY;
ALTER TABLE auth.budget_onboardings FORCE ROW LEVEL SECURITY;
CREATE POLICY budget_onboardings_read ON auth.budget_onboardings FOR SELECT USING(
 user_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid
);
CREATE POLICY budget_onboardings_insert ON auth.budget_onboardings FOR INSERT WITH CHECK(
 user_id=NULLIF(current_setting('zerobudget.user_id',true),'')::uuid
 AND EXISTS(SELECT 1 FROM auth.users u WHERE u.id=user_id AND u.enabled)
);
CREATE TRIGGER immutable_budget_onboardings BEFORE UPDATE OR DELETE ON auth.budget_onboardings FOR EACH ROW EXECUTE FUNCTION banking.immutable();
ALTER TABLE auth.audit_events DROP CONSTRAINT audit_events_action_check;
ALTER TABLE auth.audit_events DROP CONSTRAINT audit_events_check;
ALTER TABLE auth.audit_events ADD CONSTRAINT audit_events_action_check CHECK(action IN('grant','revoke','enable','disable','onboard'));
ALTER TABLE auth.audit_events ADD CONSTRAINT audit_events_check CHECK(
 (action IN('grant','revoke') AND permission IS NOT NULL)
 OR(action IN('enable','disable','onboard') AND permission IS NULL)
);

-- Exposes only the caller's enrollment state, never administrative audit rows.
CREATE FUNCTION auth.budget_onboarding_state(p_user uuid,p_issuer text,p_subject text)
RETURNS TABLE(completed_at timestamptz,managed boolean)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE account_enabled boolean;completion timestamptz;
BEGIN
 IF p_user IS DISTINCT FROM NULLIF(current_setting('zerobudget.user_id',true),'')::uuid THEN
  RAISE EXCEPTION 'identity mismatch' USING ERRCODE='42501';
 END IF;
 SELECT u.enabled INTO account_enabled FROM auth.users u WHERE u.id=p_user AND u.issuer=p_issuer AND u.subject=p_subject;
 IF NOT FOUND THEN RAISE EXCEPTION 'identity unavailable' USING ERRCODE='42501'; END IF;
 IF NOT account_enabled THEN RAISE EXCEPTION 'account disabled' USING ERRCODE='42501'; END IF;
 SELECT b.completed_at INTO completion FROM auth.budget_onboardings b WHERE b.user_id=p_user;
 RETURN QUERY SELECT completion,completion IS NULL AND (
  EXISTS(SELECT 1 FROM auth.permissions p WHERE p.user_id=p_user)
  OR EXISTS(SELECT 1 FROM auth.audit_events a WHERE a.user_id=p_user)
 );
END; $$;
REVOKE ALL ON FUNCTION auth.budget_onboarding_state(uuid,text,text) FROM PUBLIC;

CREATE FUNCTION auth.complete_budget_onboarding(p_user uuid,p_issuer text,p_subject text)
RETURNS void LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE account_enabled boolean;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' THEN RAISE EXCEPTION 'onboarding requires read committed' USING ERRCODE='25000'; END IF;
 IF p_user IS DISTINCT FROM NULLIF(current_setting('zerobudget.user_id',true),'')::uuid THEN
  RAISE EXCEPTION 'identity mismatch' USING ERRCODE='42501';
 END IF;
 -- Same row lock as administrative permission changes and account blocking.
 SELECT u.enabled INTO account_enabled FROM auth.users u WHERE u.id=p_user AND u.issuer=p_issuer AND u.subject=p_subject FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'identity unavailable' USING ERRCODE='42501'; END IF;
 IF NOT account_enabled THEN RAISE EXCEPTION 'account disabled' USING ERRCODE='42501'; END IF;
 -- A completed enrollment is never repaired, even after permissions are revoked.
 IF EXISTS(SELECT 1 FROM auth.budget_onboardings b WHERE b.user_id=p_user) THEN RETURN; END IF;
 IF EXISTS(SELECT 1 FROM auth.permissions p WHERE p.user_id=p_user)
 OR EXISTS(SELECT 1 FROM auth.audit_events a WHERE a.user_id=p_user) THEN
  RAISE EXCEPTION 'account access managed by operator' USING ERRCODE='ZO001';
 END IF;
 INSERT INTO auth.budget_onboardings(user_id) VALUES(p_user);
 INSERT INTO auth.permissions(user_id,permission) VALUES(p_user,'budget:read'),(p_user,'budget:write');
 INSERT INTO auth.audit_events(id,user_id,action,permission) VALUES
  (gen_random_uuid(),p_user,'grant','budget:read'),
  (gen_random_uuid(),p_user,'grant','budget:write'),
  (gen_random_uuid(),p_user,'onboard',NULL);
END; $$;
REVOKE ALL ON FUNCTION auth.complete_budget_onboarding(uuid,text,text) FROM PUBLIC;
