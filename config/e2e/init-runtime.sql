-- Only the isolated zerobudget_e2e database.
CREATE ROLE zb_e2e_runtime LOGIN PASSWORD 'e2e-runtime-password'
    NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT;
GRANT CONNECT ON DATABASE zerobudget_e2e TO zb_e2e_runtime;
