/*
 * Execute no console de http://localhost:3000/auth/session após login pelo Keycloak.
 * Requer budget:read e budget:write e um ciclo planejado com pelo menos 2 centavos
 * disponíveis. Use um ciclo de teste sem outras edições durante a execução.
 * Cria e remove somente seus próprios envelopes de teste. Não cria ciclos.
 */
(async () => {
  'use strict';
  if (!/^https?:$/.test(location.protocol) || location.pathname !== '/auth/session') {
    throw new Error('Execute este script na página /auth/session da API.');
  }
  const supplied = window.prompt('ID do ciclo de teste (UUID):');
  if (supplied === null) return;
  const cycleId = supplied.trim();
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(cycleId)) {
    throw new Error('Informe um UUID de ciclo válido.');
  }

  const base = `/api/cycles/${cycleId}/envelopes`;
  const cyclePath = `/api/cycles/${cycleId}`;
  const createdIds = new Set();
  const results = [];
  let csrf;
  const assert = (condition, message) => {
    if (!condition) throw new Error(message);
  };
  async function call(method, path, body, withCsrf = true) {
    const headers = {};
    if (body !== undefined) headers['Content-Type'] = 'application/json';
    if (method !== 'GET' && withCsrf && csrf) headers['X-CSRF-Token'] = csrf;
    const response = await fetch(path, {
      method, headers, credentials: 'same-origin', redirect: 'error',
      signal: AbortSignal.timeout(15000),
      ...(body === undefined ? {} : { body: JSON.stringify(body) })
    });
    const text = await response.text();
    let data;
    try {
      data = text ? JSON.parse(text) : null;
    } catch (parseError) {
      // Rejeições dos extractors do Axum (JSON inválido/campos desconhecidos)
      // podem ter corpo text/plain. O status ainda permite validar o erro.
      if (response.status < 400) {
        throw new Error(`${method} ${path}: resposta de sucesso não é JSON (HTTP ${response.status}).`);
      }
      data = text;
    }
    // Registra toda criação confirmada, mesmo se o status esperado do teste falhar.
    if (method === 'POST' && path === base && response.status === 201 && data?.id) {
      createdIds.add(data.id);
    }
    if (method === 'DELETE' && response.status === 204) {
      createdIds.delete(path.slice(path.lastIndexOf('/') + 1));
    }
    return { status: response.status, data };
  }
  async function expect(label, method, path, body, status, withCsrf = true) {
    const result = await call(method, path, body, withCsrf);
    assert(result.status === status, `${label}: esperado HTTP ${status}, recebido ${result.status}.`);
    results.push({ teste: label, resultado: 'OK', HTTP: result.status });
    return result.data;
  }
  const money = (value) => Number.isSafeInteger(value) && value >= 0;
  let initial;
  let failure;
  try {
    const session = await expect('Sessão ativa', 'GET', '/api/auth/me', undefined, 200);
    assert(['budget:read', 'budget:write'].every(p => session.permissions?.includes(p)),
      'O usuário precisa de budget:read e budget:write. Conceda as permissões pela CLI administrativa.');
    assert(typeof session.csrf_token === 'string' && session.csrf_token.length > 0,
      'Sessão sem token CSRF.');
    csrf = session.csrf_token;
    initial = await expect('Consultar ciclo', 'GET', cyclePath, undefined, 200);
    assert(initial.status === 'planned', 'O ciclo precisa estar planejado.');
    assert([initial.income_cents, initial.allocated_cents, initial.available_cents].every(money),
      'Este script exige valores inteiros seguros em JavaScript. Para valores maiores, use a suíte Rust.');
    assert(initial.available_cents >= 2, 'O ciclo precisa ter ao menos 2 centavos disponíveis.');
    assert(initial.allocated_cents + initial.available_cents === initial.income_cents,
      'O saldo inicial do ciclo é inconsistente.');
    const available = initial.available_cents;
    const first = Math.floor(available / 2);
    const second = available - first;
    const prefix = `Teste-${crypto.randomUUID()}`;
    const nameA = `${prefix}-A`;
    const nameB = `${prefix}-B`;
    const a = await expect('Criar alocação parcial', 'POST', base,
      { name: nameA, allocated_cents: first }, 201);
    const b = await expect('Distribuir saldo restante', 'POST', base,
      { name: nameB, allocated_cents: second }, 201);
    async function balance(label, allocated, remaining) {
      const value = await expect(label, 'GET', cyclePath, undefined, 200);
      assert(value.allocated_cents === allocated && value.available_cents === remaining,
        `${label}: totais diferentes do esperado; confira se houve outra edição no ciclo.`);
    }
    await balance('Toda renda distribuída', initial.income_cents, 0);
    const listed = await expect('Listar envelopes', 'GET', `${base}?limit=100`, undefined, 200);
    assert(Array.isArray(listed), 'A listagem precisa retornar um array.');
    await expect('Recusar excesso de alocação', 'POST', base,
      { name: `${prefix}-Excesso`, allocated_cents: 1 }, 409);
    await expect('Recusar nome duplicado', 'POST', base,
      { name: nameA, allocated_cents: 0 }, 409);
    await expect('Recusar valor negativo', 'POST', base,
      { name: `${prefix}-Negativo`, allocated_cents: -1 }, 422);
    await expect('Recusar nome vazio', 'POST', base,
      { name: ' ', allocated_cents: 0 }, 422);
    await expect('Recusar proprietário no corpo', 'POST', base,
      { name: `${prefix}-Proprietário`, allocated_cents: 0, owner_id: session.user_id }, 422);
    await expect('Exigir CSRF', 'POST', base,
      { name: `${prefix}-CSRF`, allocated_cents: 0 }, 403, false);
    await expect('Recusar alteração vazia', 'PATCH', `${base}/${a.id}`, {}, 422);
    await expect('Recusar null na alteração', 'PATCH', `${base}/${a.id}`, { allocated_cents: null }, 422);
    await expect('Reduzir alocação', 'PATCH', `${base}/${a.id}`, { allocated_cents: 0 }, 200);
    await balance('Saldo liberado pela redução', initial.income_cents - first, first);
    await expect('Recusar aumento acima do saldo', 'PATCH', `${base}/${a.id}`,
      { allocated_cents: first + 1 }, 409);
    await expect('Recusar renomeação duplicada', 'PATCH', `${base}/${a.id}`, { name: nameB }, 409);
    await expect('Renomear envelope', 'PATCH', `${base}/${a.id}`, { name: `${prefix}-Renomeado` }, 200);
    await expect('Remover segundo envelope', 'DELETE', `${base}/${b.id}`, undefined, 204);
    await balance('Saldo liberado pela remoção', initial.allocated_cents, available);
    await expect('Reaplicar alocação', 'PATCH', `${base}/${a.id}`, { allocated_cents: first }, 200);
    await expect('Remover primeiro envelope', 'DELETE', `${base}/${a.id}`, undefined, 204);
    await expect('Envelope removido retorna 404', 'DELETE', `${base}/${a.id}`, undefined, 404);
    await balance('Saldo original restaurado', initial.allocated_cents, available);

    // Aguarda ambas as requisições inclusive quando uma falha na rede, para que
    // a limpeza não comece enquanto uma criação confirmável ainda está em curso.
    const concurrent = await Promise.allSettled(['C', 'D'].map(suffix => call('POST', base,
      { name: `${prefix}-${suffix}`, allocated_cents: available })));
    const transportFailure = concurrent.find(result => result.status === 'rejected');
    if (transportFailure) throw transportFailure.reason;
    const statuses = concurrent.map(result => result.value.status).sort((x, y) => x - y);
    assert(statuses[0] === 201 && statuses[1] === 409,
      `Concorrência: esperado 201 e 409, recebido ${statuses.join(' e ')}.`);
    results.push({ teste: 'Concorrência sem exceder a renda', resultado: 'OK', HTTP: '201 / 409' });
    await balance('Concorrência preserva limite', initial.income_cents, 0);
  } catch (error) {
    failure = error;
  } finally {
    for (const id of [...createdIds]) {
      try {
        const result = await call('DELETE', `${base}/${id}`);
        assert(result.status === 204 || result.status === 404,
          `Limpeza retornou HTTP ${result.status}.`);
        createdIds.delete(id);
      } catch (error) {
        console.error(`Não foi possível limpar envelope de teste ${id}: ${error.message}`);
        failure ??= error;
      }
    }
    if (initial && csrf) {
      try {
        const result = await call('GET', cyclePath);
        assert(result.status === 200 && result.data.allocated_cents === initial.allocated_cents &&
          result.data.available_cents === initial.available_cents, 'Saldo final difere do inicial.');
      } catch (error) { failure ??= error; }
    }
    console.table(results);
  }
  if (failure) throw failure;
  console.log('Testes de envelopes concluídos. Saldo original preservado e envelopes de teste removidos.');
})().catch(error => {
  console.error(`Teste interrompido: ${error.message}`);
  console.warn('Se uma criação foi confirmada pelo servidor mas sua resposta se perdeu, confira os envelopes com prefixo Teste- no ciclo.');
});
