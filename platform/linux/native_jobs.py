"""Nonblocking native-message transport with one short-lived idle channel.

Producers release successful jobs; cancellation and failures close the channel.
Payload semantics remain with the producers.
"""
import json
import os
import signal
import struct
import subprocess
import time
import uuid
from gi.repository import GLib

MAX_FRAME = 256 * 1024


class NativeJobs:
    def __init__(self, launcher, idle_timeout_ms=120000, *, cwd=None, process_group=False, env=None, before_start=None, stderr_path=None):
        self.command = list(launcher) if isinstance(launcher, (list, tuple)) else [str(launcher)]
        self.cwd, self.process_group = cwd, process_group
        self.env, self.before_start = env, before_start
        self.stderr_path = stderr_path
        self.idle_timeout_ms = idle_timeout_ms
        self.children = set()
        self.reapers = {}
        self.jobs = {}
        self.channels = {}

    def start(self, payload, on_message, on_error, timeout_ms=130000):
        if self.before_start:
            self.before_start()
        identity = uuid.uuid4().hex
        request = json.dumps({**payload, 'id': identity}).encode()
        if len(request) > MAX_FRAME:
            raise ValueError('消息过长。')
        channel = next((c for c in self.channels.values() if c['job'] is None and c['proc'].poll() is None), None)
        if channel is None:
            log = None
            try:
                if self.stderr_path:
                    log = os.fdopen(os.open(self.stderr_path, os.O_WRONLY | os.O_CREAT | os.O_APPEND | os.O_NOFOLLOW, 0o600), 'ab')
                    if log.tell() > 1024*1024:
                        log.seek(0)
                        log.truncate()
                proc = subprocess.Popen(self.command, cwd=self.cwd, env=self.env, start_new_session=self.process_group,
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=log if log else subprocess.DEVNULL)
            finally:
                if log:
                    log.close()
            self.children.add(proc)
            os.set_blocking(proc.stdin.fileno(), False)
            os.set_blocking(proc.stdout.fileno(), False)
            channel = {'proc': proc, 'data': bytearray(), 'job': None}
            self.channels[proc.pid] = channel
            channel['watch'] = GLib.io_add_watch(proc.stdout.fileno(), GLib.PRIORITY_DEFAULT, GLib.IO_IN | GLib.IO_HUP | GLib.IO_ERR,
                                                lambda fd, condition: self.read(channel, fd))
        else:
            self.remove_sources(channel, 'idle_timer')
        proc = channel['proc']
        job = {'proc': proc, 'channel': channel, 'id': identity, 'message': on_message, 'error': on_error}
        channel['job'] = job
        self.jobs[identity] = job
        try:
            channel['outgoing'] = struct.pack('=I', len(request)) + request
            channel['offset'] = 0
            channel['write_watch'] = GLib.io_add_watch(proc.stdin.fileno(), GLib.PRIORITY_DEFAULT, GLib.IO_OUT | GLib.IO_HUP | GLib.IO_ERR,
                                                      lambda fd, condition: self.write(channel, fd))
            job['timer'] = GLib.timeout_add(timeout_ms, lambda: self.fail(channel, '连接超时，请重试。'))
        except (OSError, ValueError, TypeError):
            self.stop(job)
            raise
        return job

    @staticmethod
    def remove_sources(owner, *names):
        for name in names:
            source = owner.pop(name, None)
            if source:
                GLib.source_remove(source)

    def write(self, channel, fd):
        try:
            size = os.write(fd, channel['outgoing'][channel['offset']:])
            if size == 0:
                raise OSError('Connection closed')
            channel['offset'] += size
            if channel['offset'] == len(channel['outgoing']):
                channel.pop('write_watch', None)
                channel.pop('outgoing')
                return False
        except BlockingIOError:
            pass
        except OSError:
            return self.fail(channel, '连接已关闭，请重试。')
        return True

    def read(self, channel, fd):
        if channel['proc'].pid not in self.channels:
            return False
        try:
            chunk = os.read(fd, 65536)
            if not chunk:
                raise ValueError('连接已关闭，请重试。')
            data = channel['data']
            data.extend(chunk)
            while len(data) >= 4:
                size = struct.unpack('=I', data[:4])[0]
                if not 0 < size <= MAX_FRAME:
                    raise ValueError('返回的数据格式无效。')
                if len(data) < size + 4:
                    break
                event = json.loads(data[4:size+4])
                del data[:size+4]
                if not isinstance(event, dict):
                    raise ValueError('返回的数据格式无效。')
                job = channel['job']
                # A released request may still have late frames. Never route
                # them to the next request using this same process.
                if job and event.get('id') == job['id']:
                    job['message'](event)
                if channel['proc'].pid not in self.channels:
                    return False
        except BlockingIOError:
            pass
        except (OSError, ValueError, KeyError, TypeError) as error:
            return self.fail(channel, str(error))
        return True

    def fail(self, channel, error):
        job = channel['job']
        callback = job['error'] if job else None
        self.close_channel(channel)
        if callback:
            callback(error)
        return False

    def finish_job(self, job):
        self.jobs.pop(job['id'], None)
        self.remove_sources(job, 'timer')
        job['channel']['job'] = None
        job.pop('message', None)
        job.pop('error', None)

    def release(self, job):
        if not job or job['id'] not in self.jobs:
            return
        channel = job['channel']
        self.finish_job(job)
        if not self.idle_timeout_ms or channel['proc'].poll() is not None:
            self.close_channel(channel)
            return
        for other in list(self.channels.values()):
            if other is not channel and other['job'] is None:
                self.close_channel(other)
        def expire():
            channel.pop('idle_timer', None)
            self.close_channel(channel)
            return False
        channel['idle_timer'] = GLib.timeout_add(self.idle_timeout_ms, expire)

    def stop(self, job):
        if job and job['id'] in self.jobs:
            self.close_channel(job['channel'])

    def close_channel(self, channel):
        proc = channel['proc']
        if self.channels.pop(proc.pid, None) is None:
            return
        if channel['job']:
            self.finish_job(channel['job'])
        self.remove_sources(channel, 'watch', 'write_watch', 'idle_timer')
        channel['data'].clear()
        channel.pop('outgoing', None)
        proc.stdin.close()
        proc.stdout.close()
        if self.process_group:
            self.signal_group(proc, signal.SIGTERM)
        deadline = time.monotonic() + (.5 if self.process_group else 4)
        def reap():
            # A group leader can exit before its descendants. Send KILL after
            # the grace period even when the leader has already been reaped.
            if self.process_group and time.monotonic() >= deadline:
                self.signal_group(proc, signal.SIGKILL)
                if proc.poll() is None:
                    return True
                self.children.discard(proc)
                self.reapers.pop(proc.pid, None)
                return False
            if proc.poll() is not None:
                if self.process_group:
                    return True
                self.children.discard(proc)
                self.reapers.pop(proc.pid, None)
                return False
            if time.monotonic() > deadline:
                proc.kill()
            return True
        self.reapers[proc.pid] = GLib.timeout_add(100, reap)

    @staticmethod
    def signal_group(proc, sig):
        try:
            os.killpg(proc.pid, sig)
        except ProcessLookupError:
            pass

    def shutdown(self):
        for channel in list(self.channels.values()):
            self.close_channel(channel)
        for proc in list(self.children):
            source = self.reapers.pop(proc.pid, None)
            if source:
                GLib.source_remove(source)
            if self.process_group:
                self.signal_group(proc, signal.SIGKILL)
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()
        self.children.clear()
