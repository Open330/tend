import test from 'node:test';
import assert from 'node:assert/strict';
import { collaborationSummary, graphFitScale, participantLocation, responseLatency } from '../src/dashboard/web/collaboration-view.mjs';

test('history summary counts unanswered work and attention without inventing latency', () => {
  const request = {created_at:'2026-10-13T00:00:00Z', status:'completed'};
  const summary = collaborationSummary([
    {...request, reply:{at:'2026-10-13T00:00:10Z'}},
    {...request, reply:{at:'2026-10-13T00:00:30Z'}},
    {...request, status:'claimed'}, {...request, status:'blocked'},
    {...request, status:'cancelled'}, {...request, reply:{at:'invalid'}},
  ]);
  assert.deepEqual(summary, {total:6,replies:3,pending:1,attention:1,medianLatency:20000});
  assert.equal(responseLatency({...request,reply:{at:'2026-10-12T00:00:00Z'}}), null);
  assert.equal(collaborationSummary([]).medianLatency, null);
});

test('graph fit considers height as well as width and tolerates a hidden viewport', () => {
  assert.equal(graphFitScale(1000,800,1016,416),.5);
  assert.equal(graphFitScale(1000,800,516,816),.5);
  assert.equal(graphFitScale(1000,800,0,0),1);
});

test('location labels keep complete pane identifiers', () => {
  assert.equal(participantLocation({agent_kind:'claude_code',pane:'rmux:%123456'}),'rmux:%123456');
  assert.equal(participantLocation({console:true}),'operator console');
});
