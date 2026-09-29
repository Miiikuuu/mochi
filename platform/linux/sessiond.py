#!/usr/bin/env python3
"""Long-lived, model-free conversation state. Secrets and outgoing relay text live only in RAM."""
import asyncio
import fcntl
import json
import os
import re
import signal
import uuid
import time
import socket
import sys

from session_authority import AuthorityRegistry, inherited_authority, peer_pid
from session_launch import LaunchBroker

from reply_mailbox import ReplyMailbox
from session_protocol import BUILD, CAPABILITIES, MAX_FRAME, VERSION, private_directory, require_peer, socket_path


class SessionDaemon:
    def __init__(self):
        self.mailboxes = {'chatgpt': ReplyMailbox()}
        self.generation = uuid.uuid4().hex
        self.revision = 0
        self.subscribers = set()
        self.clients = 0
        self.previous = {}
        self.authority = AuthorityRegistry()
        # Only tests that start their own daemon can supply this inherited FD.
        bootstrap = inherited_authority()
        if bootstrap:
            self.authority.grants[bootstrap['capability']] = ('test', 'chatgpt', None)
        self.pushed = {}
        self.push_locks = {}

    def mailbox(self, scope):
        if not isinstance(scope, str) or not re.fullmatch(r'[a-z0-9][a-z0-9_-]{0,63}', scope):
            raise ValueError('Invalid session scope.')
        if scope not in self.mailboxes:
            if len(self.mailboxes) >= 16:
                raise ValueError('Too many session scopes.')
            self.mailboxes[scope] = ReplyMailbox()
        return self.mailboxes[scope]

    def snapshot(self, scope='chatgpt', privileged=False):
        mailbox = self.mailbox(scope)
        binding = mailbox.snapshot()  # also advances expired delivery state
        state = mailbox.connection_status()
        stable = {k: v for k, v in state.items() if k not in ('heartbeat_age', 'heartbeat_at')}
        stable['binding'] = binding
        if stable != self.previous.get(scope):
            self.revision += 1
            self.previous[scope] = stable
        value = {'version': VERSION, 'build': BUILD, 'capabilities': CAPABILITIES,
                 'generation': self.generation, 'revision': self.revision,
                 'pid': os.getpid(), 'state': state, 'binding_semantics': 'per-user'}
        state.update(service_state='active', generation=self.generation, build=BUILD, protocol=VERSION)
        if privileged:
            value['binding'] = binding
        return value

    def execute(self, request):
        operation = request.get('operation')
        scope = request.get('_scope', 'chatgpt')
        mailbox = self.mailbox(scope)
        token, client = request.get('token'), request.get('client')
        if operation in ('status', 'hello'):
            return self.snapshot(scope)
        if operation == 'binding':
            return self.snapshot(scope, privileged=True)
        if operation == 'create':
            return mailbox.create()
        if operation == 'attach':
            return mailbox.attach(token, client)
        if operation == 'poll':
            return mailbox.poll(token, client)
        if operation == 'acknowledge':
            return mailbox.acknowledge(token, client, request.get('reply_id'), request.get('state'))
        if operation == 'detach':
            return mailbox.detach(token, client)
        if operation == 'disconnect':
            mailbox.reset()
            return {'disconnected': True}
        if operation == 'submit':
            return {'reply_id': mailbox.submit(request.get('key'), request.get('text'))}
        if operation == 'observed':
            if not request.get('key') or request['key'] != mailbox.active:
                raise ValueError('Binding changed.')
            mailbox.response_received = True
            return {'accepted': True}
        if operation == 'diagnostics':
            return self.snapshot(scope)['state']
        raise ValueError('Unsupported session operation.')

    async def send(self, writer, value):
        writer.write(json.dumps(value, ensure_ascii=False, allow_nan=False).encode()+b'\n')
        await asyncio.wait_for(writer.drain(), 1)

    async def notify(self):
        async def push(writer, scope):
            # One writer at a time; requests never wait for subscribers.
            lock = self.push_locks.setdefault(writer, asyncio.Lock())
            if lock.locked():
                return
            async with lock:
                frame = self.snapshot(scope)
                revision, last = self.pushed.get(writer, (-1, 0))
                now = time.monotonic()
                if revision == frame['revision'] and now-last < 10:
                    return
                frame['event'] = 'heartbeat' if revision == frame['revision'] else 'state'
                try:
                    await self.send(writer, frame)
                    self.pushed[writer] = (frame['revision'], now)
                except (OSError, asyncio.TimeoutError):
                    self.subscribers.discard((writer, scope))
                    writer.close()
        await asyncio.gather(*(push(writer, scope) for writer, scope in tuple(self.subscribers)))

    async def handle(self, reader, writer):
        self.clients += 1
        scope = 'chatgpt'
        try:
            require_peer(writer.get_extra_info('socket'))
            if self.clients > 64:
                raise ValueError('Too many session clients.')
            raw = await asyncio.wait_for(reader.readline(), 3)
            if len(raw) > MAX_FRAME or not raw.endswith(b'\n'):
                raise ValueError('Invalid session frame.')
            request = json.loads(raw)
            if not isinstance(request, dict) or type(request.get('version')) is not int or request['version'] != VERSION:
                await self.send(writer, {'version': VERSION, 'error': {'code': 'session.version_mismatch', 'message': 'Incompatible session protocol; restart or upgrade the client and service.'}, 'build': BUILD, 'capabilities': CAPABILITIES})
                return
            scope = self.authority.authorize(request, peer_pid(writer.get_extra_info('socket')))
            request['_scope'] = scope
            self.mailbox(scope)
            if request.get('operation') == 'subscribe':
                if len(self.subscribers) >= 16:
                    raise ValueError('Too many session subscriptions.')
                self.subscribers.add((writer, scope))
                frame = self.snapshot(scope)
                await self.send(writer, frame)
                self.pushed[writer] = (frame['revision'], time.monotonic())
                await reader.read(1)
            else:
                result = self.execute(request)
                await self.send(writer, {'version': VERSION, 'result': result})
                asyncio.create_task(self.notify())
        except (ValueError, TypeError, KeyError, UnicodeError, OSError, asyncio.TimeoutError):
            try:
                await self.send(writer, {'version': VERSION, 'error': {'code': 'session.rejected', 'message': 'Session request rejected or binding expired.'}})
            except (OSError, asyncio.TimeoutError):
                pass
        finally:
            self.subscribers.discard((writer, scope))
            self.pushed.pop(writer, None)
            self.push_locks.pop(writer, None)
            self.clients -= 1
            writer.close()

    async def serve(self):
        directory = private_directory()
        with (directory/'sessiond.lock').open('a+b') as lock:
            os.chmod(directory/'sessiond.lock', 0o600)
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                return
            path = socket_path()
            activated = int(os.environ.get('LISTEN_PID', '0')) == os.getpid() and os.environ.get('LISTEN_FDS') == '1'
            if activated:
                listener = socket.socket(fileno=3)
                if listener.family != socket.AF_UNIX or listener.getsockname() != str(path):
                    raise RuntimeError('Invalid systemd activation socket.')
                server = await asyncio.start_unix_server(self.handle, sock=listener, limit=MAX_FRAME,
                    **({'cleanup_socket': False} if sys.version_info >= (3, 13) else {}))
            else:
                path.unlink(missing_ok=True)
                server = await asyncio.start_unix_server(self.handle, path=str(path), limit=MAX_FRAME)
                os.chmod(path, 0o600)
            broker = LaunchBroker(self.authority)
            stopped = asyncio.Event()
            loop = asyncio.get_running_loop()
            for signum in (signal.SIGTERM, signal.SIGINT):
                loop.add_signal_handler(signum, stopped.set)
            try:
                while not stopped.is_set():
                    try:
                        await asyncio.wait_for(stopped.wait(), 1)
                    except asyncio.TimeoutError:
                        await self.notify()
            finally:
                # Notify before a package/service restart; never report an
                # in-flight user reply as Sent after losing its generation.
                for mailbox in self.mailboxes.values():
                    if mailbox.pending and not mailbox.response_received:
                        mailbox.pending['state'] = 'uncertain'
                        mailbox.seen = float('-inf')
                await self.notify()
                await asyncio.sleep(.15)
                broker.close()
                for writer, _scope in tuple(self.subscribers):
                    writer.close()
                server.close()
                await asyncio.wait_for(server.wait_closed(), 4)
                if not activated:
                    path.unlink(missing_ok=True)


if __name__ == '__main__':
    os.umask(0o077)
    asyncio.run(SessionDaemon().serve())
