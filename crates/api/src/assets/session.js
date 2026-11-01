'use strict';
(() => {
  const status = document.getElementById('status');
  const identity = document.getElementById('identity');
  const login = document.getElementById('login');
  const refresh = document.getElementById('refresh');
  const logout = document.getElementById('logout');
  let csrfToken = null;

  function clearIdentity() {
    csrfToken = null;
    identity.hidden = true;
    for (const id of ['issuer', 'subject', 'expires', 'user-id', 'permissions']) {
      document.getElementById(id).textContent = '';
    }
    login.hidden = false;
    logout.disabled = true;
  }

  async function checkSession() {
    clearIdentity();
    const response = await fetch('/api/auth/me', { credentials: 'same-origin' });
    if (response.status === 401) {
      status.textContent = 'Sem sessão ativa (HTTP 401). Entre com Keycloak para iniciar uma sessão.';
      return response.status;
    }
    if (!response.ok) throw new Error(`Consulta retornou HTTP ${response.status}.`);
    const data = await response.json();
    if (typeof data.csrf_token !== 'string' || !data.csrf_token) {
      throw new Error('A resposta não contém a proteção necessária para sair.');
    }
    csrfToken = data.csrf_token;
    document.getElementById('user-id').textContent = data.user_id;
    document.getElementById('permissions').textContent = data.permissions.length ? data.permissions.join(', ') : 'Nenhuma permissão financeira concedida';
    document.getElementById('issuer').textContent = data.issuer;
    document.getElementById('subject').textContent = data.subject;
    document.getElementById('expires').textContent = new Date(data.expires_at * 1000).toLocaleString('pt-BR');
    identity.hidden = false;
    login.hidden = true;
    logout.disabled = false;
    status.textContent = 'Sessão ativa (HTTP 200).';
    return response.status;
  }

  function showError(error) {
    clearIdentity();
    status.textContent = `Não foi possível concluir a operação. ${error.message}`;
  }

  refresh.addEventListener('click', async () => {
    refresh.disabled = true;
    try { await checkSession(); } catch (error) { showError(error); }
    finally { refresh.disabled = false; }
  });

  logout.addEventListener('click', async () => {
    if (!csrfToken) return;
    logout.disabled = true;
    refresh.disabled = true;
    try {
      const response = await fetch('/api/auth/logout', {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'X-CSRF-Token': csrfToken }
      });
      if (response.status !== 204) throw new Error(`Logout retornou HTTP ${response.status}.`);
      const after = await checkSession();
      if (after !== 401) throw new Error('A sessão ainda aparece ativa após o logout.');
      status.textContent = 'Logout concluído (HTTP 204). Sessão revogada (HTTP 401).';
    } catch (error) { showError(error); }
    finally { refresh.disabled = false; }
  });

  checkSession().catch(showError);
})();
