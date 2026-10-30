#!/usr/bin/env python3
"""Deterministic app-server stand-in. No model calls or shell execution."""
import json
from pathlib import Path
import sys
import uuid

state_path = Path(sys.argv[0]).with_suffix('.state.json')
threads = json.loads(state_path.read_text()) if state_path.exists() else {}
approval_thread = None


def save():
    state_path.write_text(json.dumps(threads))


def send(value):
    print(json.dumps(value), flush=True)


def event(method, params):
    send({'method': method, 'params': params})


def metadata(thread):
    return {k: v for k, v in thread.items() if k != 'turns'}


def config(thread):
    return {'thread': metadata(thread), 'model': thread.get('model', 'test-model'),
            'approvalPolicy': thread.get('approvalPolicy', 'on-request'),
            'sandbox': {'type': {'read-only': 'readOnly', 'danger-full-access': 'dangerFullAccess'}.get(thread.get('sandbox'), 'workspaceWrite')},
            'reasoningEffort': thread.get('config', {}).get('model_reasoning_effort')}


for line in sys.stdin:
    message = json.loads(line)
    with Path(sys.argv[0]).with_suffix('.calls.jsonl').open('a') as log:
        log.write(json.dumps(message) + '\n')
    method = message.get('method')
    request_id = message.get('id')
    params = message.get('params', {})
    result = {}
    if method == 'initialize':
        result = {'userAgent': 'test'}
    elif method == 'initialized':
        continue
    elif method == 'model/list':
        result = {'data': [{'id': 'test-model', 'model': 'test-model', 'displayName': 'Test model'}]}
    elif method == 'thread/list':
        assert 'cwd' not in params, 'History must span all working folders'
        stored = [metadata(t) for t in threads.values() if bool(t.get('archived', False)) == bool(params.get('archived', False)) and t.get('turns') and t.get('source', 'vscode') in params.get('sourceKinds', ['cli', 'vscode'])]
        stored.sort(key=lambda t: t.get('updatedAt', 0), reverse=True)
        offset = int(params.get('cursor', 'page:0').split(':')[1])
        limit = params.get('limit', 50)
        end = offset + limit
        result = {'data': stored[offset:end], 'nextCursor': 'page:' + str(end) if end < len(stored) else None}
    elif method == 'thread/start':
        thread_id = 'thread-' + uuid.uuid4().hex[:8]
        thread = {**params, 'id': thread_id, 'name': None, 'preview': '', 'turns': [], 'status': {'type': 'idle'}}
        threads[thread_id] = thread
        save()
        result = config(thread)
    elif method == 'thread/resume':
        if params['threadId'] not in threads:
            send({'id': request_id, 'error': {'code': -32600, 'message': 'Session rollout is missing'}})
            continue
        thread = threads[params['threadId']]
        if thread.get('locked'):
            send({'id': request_id, 'error': {'code': -32603, 'message': 'thread-store conflict: thread already has an active writer'}})
            continue
        thread.update({k: v for k, v in params.items() if k not in ('threadId', 'excludeTurns')})
        save()
        result = config(thread)
    elif method == 'thread/unsubscribe':
        result = {'status': 'unsubscribed'}
    elif method == 'thread/inject_items':
        save()
    elif method == 'turn/steer':
        thread_id = params['threadId']
        turns = threads[thread_id]['turns']
        if not turns or turns[-1]['status'] != 'inProgress':
            send({'id': request_id, 'error': {'code': -32600, 'message': 'No active turn'}})
            continue
        result = {'turnId': turns[-1]['id']}
    elif method == 'turn/interrupt':
        thread_id = params['threadId']
        if threads[thread_id]['turns']:
            threads[thread_id]['turns'][-1]['status'] = 'interrupted'
            save()
            event('turn/completed', {'threadId': thread_id, 'turn': {'id': params['turnId'], 'status': 'interrupted'}})
    elif method == 'thread/read':
        result = {'thread': metadata(threads[params['threadId']])}
    elif method == 'thread/name/set':
        threads[params['threadId']]['name'] = params['name']
        save()
    elif method == 'thread/turns/list':
        result = {'data': list(reversed(threads[params['threadId']]['turns'])), 'nextCursor': None}
    elif method == 'turn/start':
        thread_id = params['threadId']
        approval_thread = thread_id
        thread = threads.setdefault(thread_id, {'id': thread_id, 'turns': []})
        user = {'id': 'user-' + uuid.uuid4().hex[:8], 'type': 'userMessage', 'content': params['input']}
        turn = {'id': 'turn-1', 'status': 'inProgress', 'items': [user]}
        thread['turns'].append(turn)
        save()
        send({'id': request_id, 'result': {'turn': turn}})
        event('turn/started', {'threadId': thread_id, 'turn': {'id': 'turn-1'}})
        event('item/completed', {'threadId': thread_id, 'item': user})
        send({'id': 100, 'method': 'item/commandExecution/requestApproval', 'params': {'threadId': thread_id, 'turnId': 'turn-1', 'itemId': 'item-1', 'command': 'echo test', 'cwd': thread.get('cwd', '/projects/test'), 'reason': 'Test approval'}})
        continue
    elif method is None and request_id == 100:
        thread_id = approval_thread
        tool = {'id': 'item-1', 'type': 'commandExecution', 'command': 'echo test', 'aggregatedOutput': 'test', 'status': 'completed'}
        agent = {'id': 'agent-1', 'type': 'agentMessage', 'text': 'Done. The worker is connected and ready.'}
        turn = threads[thread_id]['turns'][-1]
        turn['items'].extend([tool, agent])
        turn['status'] = 'completed'
        save()
        event('item/completed', {'threadId': thread_id, 'item': tool})
        event('item/started', {'threadId': thread_id, 'item': {**agent, 'text': ''}})
        event('item/agentMessage/delta', {'threadId': thread_id, 'itemId': 'agent-1', 'delta': agent['text']})
        event('item/completed', {'threadId': thread_id, 'item': agent})
        event('turn/completed', {'threadId': thread_id, 'turn': {'id': 'turn-1', 'status': 'completed'}})
        continue
    else:
        send({'id': request_id, 'error': {'code': -32601, 'message': 'Unknown test method'}})
        continue
    send({'id': request_id, 'result': result})
