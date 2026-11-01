# zerobudget

Aplicação Rust de gestão financeira pessoal com orçamento base zero.

- [Arquitetura do backend](docs/ARQUITETURA_BACKEND_RUST.md)
- [Executar a API e testar login pelo navegador](crates/api/README.md)
- [Autenticação e autorização de usuários](crates/auth/README.md)
- [Keycloak para desenvolvimento local](docs/setup-keycloak.md)

- [Entrega de sessões compartilhadas e próximos passos](docs/proximo-passo-auth.md)

- [Identidades locais e permissões persistentes](docs/identidades-e-permissoes.md)

Ciclos de orçamento: [contrato, migração e testes](docs/ciclos-planejados.md) e
[ativação e fechamento](docs/ciclo-de-vida.md).

Envelopes: [regras, API e roteiro de teste](docs/envelopes.md).

- [Contrato da API e coleção Postman](docs/api/README.md)

- [E2E com login Keycloak automático e Playwright MCP](docs/api/e2e.md): `./scripts/e2e test`

Despesas manuais e estornos: [regras, saldo e API](docs/despesas.md).

Contas bancárias manuais e carteira: [saldos, recebimentos e transferências](docs/contas.md).

Cartões e faturas manuais: [compras, limite e pagamentos](docs/cartoes.md).

- [Backend MVP: planejamento e correções](docs/planejamento-e-correcoes.md)
- [Operação, acesso ao piloto e recuperação](docs/operacao.md)
- [Checklist da entrega backend](docs/backend-mvp.md)
