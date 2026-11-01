-- PostgreSQL 18: generation policy for new local identifiers; existing keys are preserved.

CREATE OR REPLACE FUNCTION banking.record_card_payment() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
BEGIN
 INSERT INTO banking.movements(id,owner_id,kind,from_account_id,description,occurred_on,amount_cents,card_payment_id) VALUES(pg_catalog.uuidv7(),NEW.owner_id,'card_payment',NEW.account_id,NEW.description,NEW.occurred_on,NEW.amount_cents,NEW.id);
 RETURN NEW;
END; $$;

CREATE OR REPLACE FUNCTION banking.record_payment() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE x record;
BEGIN
 IF TG_TABLE_NAME='expenses' THEN
   IF NEW.account_id IS NULL THEN RETURN NEW; END IF;
   INSERT INTO banking.movements(id,owner_id,kind,from_account_id,description,occurred_on,amount_cents,expense_id)
     VALUES(pg_catalog.uuidv7(),NEW.owner_id,'expense',NEW.account_id,NEW.description,NEW.occurred_on,NEW.amount_cents,NEW.id);
 ELSE
   SELECT * INTO x FROM budgeting.expenses WHERE id=NEW.expense_id AND owner_id=NEW.owner_id;
   IF x.account_id IS NOT NULL THEN
     INSERT INTO banking.movements(id,owner_id,kind,to_account_id,description,occurred_on,amount_cents,expense_id)
       VALUES(pg_catalog.uuidv7(),NEW.owner_id,'expense_reversal',x.account_id,x.description,x.occurred_on,x.amount_cents,x.id);
   END IF;
 END IF;
 RETURN NEW;
END; $$;

CREATE OR REPLACE FUNCTION banking.record_adjustment() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE original record;
BEGIN
 SELECT * INTO original FROM banking.movements WHERE id=NEW.movement_id AND owner_id=NEW.owner_id;
 INSERT INTO banking.movements(id,owner_id,kind,from_account_id,to_account_id,description,occurred_on,amount_cents,adjustment_id) VALUES(pg_catalog.uuidv7(),NEW.owner_id,'correction',original.to_account_id,original.from_account_id,NEW.description,NEW.occurred_on,original.amount_cents,NEW.id);
 RETURN NEW;
END; $$;

CREATE OR REPLACE FUNCTION auth.complete_budget_onboarding(p_user uuid,p_issuer text,p_subject text)
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
  (pg_catalog.uuidv7(),p_user,'grant','budget:read'),
  (pg_catalog.uuidv7(),p_user,'grant','budget:write'),
  (pg_catalog.uuidv7(),p_user,'onboard',NULL);
END; $$;

ALTER TABLE auth.users ALTER COLUMN id SET DEFAULT pg_catalog.uuidv7();
ALTER TABLE auth.audit_events ALTER COLUMN id SET DEFAULT pg_catalog.uuidv7();
ALTER TABLE budgeting.cycles ALTER COLUMN id SET DEFAULT pg_catalog.uuidv7();
ALTER TABLE budgeting.envelopes ALTER COLUMN id SET DEFAULT pg_catalog.uuidv7();
ALTER TABLE budgeting.expenses ALTER COLUMN id SET DEFAULT pg_catalog.uuidv7();
ALTER TABLE banking.accounts ALTER COLUMN id SET DEFAULT pg_catalog.uuidv7();
ALTER TABLE banking.movements ALTER COLUMN id SET DEFAULT pg_catalog.uuidv7();
ALTER TABLE banking.credit_cards ALTER COLUMN id SET DEFAULT pg_catalog.uuidv7();
ALTER TABLE banking.card_invoices ALTER COLUMN id SET DEFAULT pg_catalog.uuidv7();
ALTER TABLE banking.card_payments ALTER COLUMN id SET DEFAULT pg_catalog.uuidv7();
ALTER TABLE budgeting.reallocations ALTER COLUMN id SET DEFAULT pg_catalog.uuidv7();
ALTER TABLE banking.adjustments ALTER COLUMN id SET DEFAULT pg_catalog.uuidv7();
ALTER TABLE banking.opening_debts ALTER COLUMN id SET DEFAULT pg_catalog.uuidv7();
ALTER TABLE banking.card_credits ALTER COLUMN id SET DEFAULT pg_catalog.uuidv7();

-- Insert-only policy allows historical UUIDv4 rows to remain readable/updatable.
-- Foreign keys, OIDC subjects, and opaque security secrets are not regenerated.
CREATE FUNCTION auth.require_uuid_v7() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF uuid_extract_version(NEW.id) IS DISTINCT FROM 7 THEN
  RAISE EXCEPTION 'new local identifiers must be RFC 9562 UUIDv7' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END; $$;
REVOKE ALL ON FUNCTION auth.require_uuid_v7() FROM PUBLIC;
CREATE TRIGGER require_uuid_v7 BEFORE INSERT ON auth.users FOR EACH ROW EXECUTE FUNCTION auth.require_uuid_v7();
CREATE TRIGGER require_uuid_v7 BEFORE INSERT ON auth.audit_events FOR EACH ROW EXECUTE FUNCTION auth.require_uuid_v7();
CREATE TRIGGER require_uuid_v7 BEFORE INSERT ON budgeting.cycles FOR EACH ROW EXECUTE FUNCTION auth.require_uuid_v7();
CREATE TRIGGER require_uuid_v7 BEFORE INSERT ON budgeting.envelopes FOR EACH ROW EXECUTE FUNCTION auth.require_uuid_v7();
CREATE TRIGGER require_uuid_v7 BEFORE INSERT ON budgeting.expenses FOR EACH ROW EXECUTE FUNCTION auth.require_uuid_v7();
CREATE TRIGGER require_uuid_v7 BEFORE INSERT ON banking.accounts FOR EACH ROW EXECUTE FUNCTION auth.require_uuid_v7();
CREATE TRIGGER require_uuid_v7 BEFORE INSERT ON banking.movements FOR EACH ROW EXECUTE FUNCTION auth.require_uuid_v7();
CREATE TRIGGER require_uuid_v7 BEFORE INSERT ON banking.credit_cards FOR EACH ROW EXECUTE FUNCTION auth.require_uuid_v7();
CREATE TRIGGER require_uuid_v7 BEFORE INSERT ON banking.card_invoices FOR EACH ROW EXECUTE FUNCTION auth.require_uuid_v7();
CREATE TRIGGER require_uuid_v7 BEFORE INSERT ON banking.card_payments FOR EACH ROW EXECUTE FUNCTION auth.require_uuid_v7();
CREATE TRIGGER require_uuid_v7 BEFORE INSERT ON budgeting.reallocations FOR EACH ROW EXECUTE FUNCTION auth.require_uuid_v7();
CREATE TRIGGER require_uuid_v7 BEFORE INSERT ON banking.adjustments FOR EACH ROW EXECUTE FUNCTION auth.require_uuid_v7();
CREATE TRIGGER require_uuid_v7 BEFORE INSERT ON banking.opening_debts FOR EACH ROW EXECUTE FUNCTION auth.require_uuid_v7();
CREATE TRIGGER require_uuid_v7 BEFORE INSERT ON banking.card_credits FOR EACH ROW EXECUTE FUNCTION auth.require_uuid_v7();
