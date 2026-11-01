#!/usr/bin/env python3
"""Only reads the E2E source; creates and drops its own empty restore target."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
from urllib.parse import urlparse, unquote
import uuid

repo = Path(__file__).resolve().parents[2]
u = urlparse(os.environ['AUTH_TEST_DATABASE_URL'])
if u.hostname not in ('localhost', '127.0.0.1') or u.path != '/zerobudget_e2e':
    raise SystemExit('Use somente o banco isolado zerobudget_e2e em loopback.')
env = {**os.environ, 'PGHOST':u.hostname, 'PGPORT':str(u.port or 5432),
       'PGUSER':unquote(u.username), 'PGPASSWORD':unquote(u.password),
       'PGDATABASE':'zerobudget_e2e'}
target = 'zb_restore_test_' + uuid.uuid4().hex

def run(args, db='zerobudget_e2e', ok=True):
    result = subprocess.run(args, cwd=repo, env={**env,'PGDATABASE':db},
                            stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    if ok and result.returncode:
        raise RuntimeError('Falhou: ' + str(args[0]) + ' (diagnóstico privado omitido)')
    return result

def sql(query, db='zerobudget_e2e'):
    return run(['psql','-XAt','--set=ON_ERROR_STOP=1','--command',query],db).stdout.strip()

def fingerprint(db):
    tables = sql("SELECT quote_ident(n.nspname)||'.'||quote_ident(c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind='r' AND n.nspname IN ('auth','budgeting','banking','public') ORDER BY 1",db).splitlines()
    digest = hashlib.sha256()
    for table in tables:
        value = sql(f"SELECT COALESCE(string_agg(row_to_json(t)::text,E'\\n' ORDER BY row_to_json(t)::text),'') FROM {table} t",db)
        digest.update((table+'\n'+value).encode())
    return digest.hexdigest(), len(tables)

created = False
try:
    sql(f'CREATE DATABASE "{target}"'); created = True
    with tempfile.TemporaryDirectory(prefix='zb-backup-') as directory:
        backup = str(Path(directory)/'test.dump')
        run(['scripts/db-backup',backup])
        assert (Path(backup).stat().st_mode & 0o777)==0o600
        assert run(['scripts/db-backup',backup],ok=False).returncode != 0
        assert run(['scripts/db-restore',backup,'wrong_name'],target,ok=False).returncode != 0
        run(['scripts/db-restore',backup,target],target)
        before, count = fingerprint('zerobudget_e2e')
        assert fingerprint(target) == (before,count), 'Histórico financeiro divergiu'
        assert run(['scripts/db-restore',backup,target],target,ok=False).returncode != 0
        public_functions = sql("SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace CROSS JOIN LATERAL aclexplode(COALESCE(p.proacl,acldefault('f',p.proowner))) a WHERE n.nspname IN('auth','budgeting','banking') AND a.grantee=0 AND a.privilege_type='EXECUTE'", target)
        assert public_functions == '0'
        runtime = urlparse(os.environ['AUTH_TEST_RUNTIME_DATABASE_URL'])
        migration_env = {**env,'AUTH_ALLOW_LOCAL_HTTP':'true',
                         'DATABASE_ADMIN_URL':u._replace(path='/'+target).geturl(),
                         'DATABASE_RUNTIME_ROLE':unquote(runtime.username)}
        migrated = subprocess.run([os.environ['ZB_MIGRATE_BIN']],env=migration_env,
                                   stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        assert migrated.returncode == 0, 'Falha ao reaplicar privilégios mínimos'
        assert fingerprint(target) == (before,count), 'Migração alterou histórico restaurado'
        runtime_env = {**env,'PGDATABASE':target,'PGUSER':unquote(runtime.username),
                       'PGPASSWORD':unquote(runtime.password)}
        user = sql("SELECT id FROM auth.users u WHERE enabled AND EXISTS(SELECT 1 FROM auth.permissions p WHERE p.user_id=u.id AND p.permission='budget:read') ORDER BY id LIMIT 1",target)
        assert user, 'Execute E2E financeiro antes de testar recuperação'
        query = f"BEGIN; SELECT set_config('zerobudget.user_id','{user}',true); SELECT count(*) FROM banking.card_positions; SELECT count(*) FROM budgeting.envelope_balances; ROLLBACK;"
        check = subprocess.run(['psql','-XAt','--set=ON_ERROR_STOP=1','--command',query],env=runtime_env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        assert check.returncode == 0, 'Runtime restaurado sem acesso às projeções'
        print(f'Backup/restauração OK: {count} tabelas, dados e histórico idênticos; recusa de sobrescrita/destino não vazio; funções sem EXECUTE público.')
finally:
    if created:
        sql(f'DROP DATABASE "{target}" WITH (FORCE)')
