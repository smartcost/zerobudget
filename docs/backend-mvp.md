# Entrega do backend MVP

Estado consolidado em 01/11/2026: escopo funcional implementado e validado.
Interface, integração bancária, parcelamento e juros continuam fora desta entrega.
A publicação do piloto e a validação da imagem final ainda não foram executadas.

## Funcionalidades entregues

- [x] Login OIDC com PKCE, sessões no servidor, CSRF, autorização local e isolamento por proprietário/RLS.
- [x] Signup, confirmação de e-mail e recuperação de senha pelo Keycloak; revogação de sessões anteriores ao trocar senha.
- [x] Onboarding financeiro explícito, auditado e idempotente; progresso inicial sem restaurar permissões revogadas.
- [x] Ciclos planejados/ativos/fechados, distribuição em envelopes e ativação com saldo zero.
- [x] Contas manuais, recebimentos e transferências com saldo exato e proteção contra concorrência.
- [x] Despesas, estornos, cartões, faturas e pagamentos sem duplicar o consumo do orçamento.
- [x] Realocação com histórico e saldo já gasto preservado.
- [x] Idempotência e compensação integral de recebimentos, transferências e pagamentos.
- [x] Crédito de devolução de compra fechada e dívida inicial do cartão sem receita/despesa fictícia.
- [x] Resumos de ciclo e posição financeira sob snapshot consistente.
- [x] Próximo ciclo: vazio, cópia ou transporte de saldo sem duplicar receita.
- [x] UUIDv7 para novos IDs locais, request_id e geração SQL; referências históricas preservadas.
- [x] OpenAPI, Postman, Playwright com Keycloak real e integrações PostgreSQL/Redis.
- [x] Readiness, logs, artefatos de implantação e backup/restauração testados.
- [x] Runbook de onboarding, administração de acesso e recuperação.

Login/cadastro não concedem direitos financeiros. O usuário elegível os habilita
uma única vez com POST de onboarding e CSRF. Revogações e decisões administrativas
prevalecem. Correções são imutáveis; replay idêntico não duplica efeitos; crédito
no cartão não é caixa; transporte exige origem fechada e um único sucessor.

## Evidências de validação

A implementação UUIDv7 passou em `./scripts/e2e verify`: 60 cenários Playwright com
Keycloak real, 88 testes Rust e 2 doctests, incluindo PostgreSQL/Redis. A prova
adicional de migração com identidade/referências UUIDv4 passou separadamente,
totalizando 89 testes Rust. Backup/restauração comparou as 18 tabelas, histórico e
privilégios runtime. Formatação, Clippy com warnings como erro, tipos TypeScript e
os 11 geradores UUIDv7 da coleção Postman também foram verificados.

A migração 0011 foi aplicada no banco local em 01/11/2026 após backup validado.
As 14 tabelas com chave própria possuem validação de UUIDv7 nos INSERTs. Nenhum
ID histórico foi convertido e não houve concessão de permissões financeiras a
usuários nem ampliação de privilégios runtime nesta consolidação.

O contrato agora exige UUIDv7 em request_id: v4 retorna 422, inclusive em retries
antigos. Clientes devem concluir operações pendentes antes da atualização e
preservar a mesma chave v7 em retries. Leia [compatibilidade UUIDv7](postgresql-18.md).
Reinicie a API para carregar o código atualizado; a migração não reinicia processos.

A imagem `zerobudget:mvp-local` foi compilada e verificada na etapa de onboarding
(UID 10001 e ferramentas administrativas). Essa evidência antecede UUIDv7: a imagem
final precisa ser reconstruída e submetida a smoke test com credenciais runtime.
O Compose de implantação foi validado sintaticamente; não equivale a implantação.

## Pendências para liberar o piloto

- [ ] Reconstruir a imagem final e validar readiness, login/onboarding e operação financeira usando o runtime.
- [ ] Provisionar domínio, HTTPS, segredos e serviços PostgreSQL/Redis/Keycloak no ambiente escolhido.
- [ ] Configurar SMTP real e validar entrega de confirmação de e-mail e recuperação de senha.
- [ ] Executar a jornada completa no ambiente publicado e verificar isolamento entre participantes.
- [ ] Agendar backups, definir retenção, testar recuperação no ambiente e configurar monitoramento/alertas.

Benchmark e tuning adicional do PostgreSQL não são critérios de conclusão
funcional: nenhum ganho percentual foi atribuído sem medição representativa.
A interface será discutida separadamente.

Guias: [onboarding](onboarding.md), [Keycloak](setup-keycloak.md),
[regras e contratos](planejamento-e-correcoes.md), [PostgreSQL 18](postgresql-18.md),
[operação e recuperação](operacao.md) e [execução E2E](api/e2e.md).
