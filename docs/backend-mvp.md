# Entrega do backend MVP

Escopo aprovado: realocação; correções e idempotência; resumo consistente;
entrada de dívida inicial; próximo ciclo; preparação operacional. Interface fora
desta entrega, a discutir separadamente. Integração bancária, parcelas e juros
continuam fora do escopo.

Implementado e validado em 31/10/2026:
- [x] Realocação com histórico e saldo já gasto preservado.
- [x] Idempotência de despesas, recebimentos e transferências.
- [x] Compensação de recebimentos, transferências e pagamentos.
- [x] Crédito de devolução de compra fechada em fatura aberta, sem reescrever o ciclo.
- [x] Saldo inicial de dívida do cartão, sem despesa/renda fictícia.
- [x] Resumos de ciclo e posição financeira sob snapshot consistente.
- [x] Próximo ciclo: vazio, cópia ou transporte de saldo; sem duplicar receita.
- [x] Readiness, logs, artefatos de implantação e backup/restauração testados.
- [x] Runbook de ingresso e concessão explícita de acesso ao piloto.
- [x] Contratos, Postman, regressões e migrações locais.

Regras: correções são integrais e imutáveis; pagamento repetido não duplica efeito;
transporte exige ciclo de origem fechado e um único sucessor. Crédito de devolução
não é dinheiro em conta e não altera o orçamento de um ciclo já fechado.


Validação: `cargo fmt --all --check`, Clippy com warnings como erro e
`./scripts/e2e verify`: 55 cenários Playwright com Keycloak real, 80 testes Rust mais
2 doctests, incluindo integrações PostgreSQL/Redis, e backup/restauração das 17
tabelas com comparação do conteúdo e reaplicação dos privilégios runtime.
Imagem `zerobudget:mvp-local` compilada; UID 10001 e ferramentas administrativas
verificados. Compose de implantação validado sintaticamente. Migrações 0008/0009
aplicadas no banco local, sem concessão de permissões financeiras a usuários.

Guias: [regras e contratos](planejamento-e-correcoes.md),
[operação e recuperação](operacao.md), [execução E2E](api/e2e.md).
A API em execução precisa ser reiniciada para carregar as novas rotas.
Publicação por HTTPS, DNS e segredos de produção dependem do ambiente escolhido;
a implantação pública e a interface não foram executadas nesta entrega.

Evolução de autenticação: Keycloak substituiu o Dex; cadastro, verificação de e-mail
e recuperação estão em [setup-keycloak](setup-keycloak.md). A regressão passa a
incluir 55 cenários E2E. Não existe migração das identidades descartáveis anteriores.
