const fs = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const test = require('node:test');
const path = require('node:path');
const source = fs.readFileSync(path.join(__dirname, 'test-envelopes.js'), 'utf8');
test('o arquivo completo tem sintaxe JavaScript válida', () => {
  assert.doesNotThrow(() => new vm.Script(source));
});
async function run(fail) {
  const cycle = '11111111-1111-4111-8111-111111111111';
  const base = `/api/cycles/${cycle}/envelopes`;
  const records = new Map(); let counter=0, injected=false;
  const logs=[];
  const respond=(status,data)=>({status,text:async()=>status===422?'Failed to deserialize the JSON body: unknown field':data===undefined?'':JSON.stringify(data)});
  await vm.runInNewContext(source, {
    location:{protocol:'http:',pathname:'/auth/session'},
    window:{prompt:()=>cycle}, crypto:{randomUUID:()=> 'unique-run'}, AbortSignal,
    console:{table:value=>logs.push(value),log:value=>logs.push(value),error:value=>logs.push(value),warn:value=>logs.push(value)},
    fetch:async(path,options)=> {
      const method=options.method;
      if(path==='/api/auth/me') return respond(200,{csrf_token:'private-token',user_id:'owner',permissions:['budget:read','budget:write']});
      if(path===`/api/cycles/${cycle}`) {
        if(fail && records.size===2 && !injected) {injected=true;return respond(503,{});}
        const allocated=1000+[...records.values()].reduce((sum,e)=>sum+e.allocated_cents,0);
        return respond(200,{status:'planned',income_cents:10000,allocated_cents:allocated,available_cents:10000-allocated});
      }
      const body=options.body?JSON.parse(options.body):undefined;
      if(method==='GET') return respond(200,[...records.values()]);
      if(!options.headers['X-CSRF-Token']) return respond(403,{});
      assert.equal(options.headers['X-CSRF-Token'],'private-token');
      const id=path.slice(path.lastIndexOf('/')+1);
      if(method==='DELETE') {if(!records.delete(id))return respond(404,{});return respond(204);}
      if(body.owner_id || body.name===null || body.allocated_cents===null || body.allocated_cents<0 || body.name?.trim()===' ' || body.name?.trim()==='' || Object.keys(body).length===0) return respond(422,{});
      if(method==='PATCH' && !records.has(id))return respond(404,{});
      const next=method==='PATCH'?{...records.get(id),...body}:{...body,id:`created-${++counter}`};
      if([...records.values()].some(e=>e.id!==next.id && e.name===next.name))return respond(409,{});
      const allocated=[...records.values()].filter(e=>e.id!==next.id).reduce((sum,e)=>sum+e.allocated_cents,1000)+next.allocated_cents;
      if(allocated>10000)return respond(409,{});
      records.set(next.id,next);return respond(method==='PATCH'?200:201,next);
    }
  });
  assert.equal(records.size,0,'created envelopes must be cleaned');
  assert(!JSON.stringify(logs).includes('private-token'),'CSRF token must not be logged');
  assert(logs.some(v=>typeof v==='string' && v.includes(fail?'Teste interrompido:':'Testes de envelopes concluídos.')));
}
test('aceita erros 422 text/plain e conclui o roteiro com limpeza', () => run(false));
test('limpa envelopes quando uma resposta inesperada interrompe o roteiro', () => run(true));
