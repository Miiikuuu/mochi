"""Bounded RGBA stream from Mochi's isolated native renderer to a GTK image."""
import os
import struct
import subprocess
import tempfile
from gi.repository import GdkPixbuf, GLib


class ActionPreview:
    def __init__(self, binary, package, action, picture, failed):
        self.picture, self.failed = picture, failed
        self.closed = False
        self.buffer = bytearray()
        self.dimensions = None
        self.frames = 0
        self.log = tempfile.TemporaryFile()
        try:
            self.process = subprocess.Popen([str(binary), '--preview-action', package, action['id'], action['kind']],
                                            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=self.log)
        except BaseException:
            self.log.close()
            raise
        os.set_blocking(self.process.stdout.fileno(), False)
        self.watch = GLib.io_add_watch(self.process.stdout.fileno(), GLib.PRIORITY_DEFAULT,
                                      GLib.IO_IN | GLib.IO_HUP | GLib.IO_ERR, self.receive)

    def receive(self, fd, condition):
        if self.closed:
            return False
        ended = False
        try:
            # One latest complete frame; never queue frames on the GTK thread.
            for _ in range(8):
                try:
                    chunk = os.read(fd, 65536)
                except BlockingIOError:
                    break
                if not chunk:
                    ended = True
                    break
                self.buffer.extend(chunk)
            if self.dimensions is None and len(self.buffer) >= 12:
                magic, width, height = struct.unpack('<4sII', self.buffer[:12])
                if magic != b'MPR1' or (width, height) != (256, 256):
                    raise ValueError('Invalid action preview stream.')
                self.dimensions = width, height
                del self.buffer[:12]
            if self.dimensions:
                width, height = self.dimensions
                size = width*height*4
                count = len(self.buffer)//size
                if count:
                    data = GLib.Bytes.new(bytes(self.buffer[(count-1)*size:count*size]))
                    del self.buffer[:count*size]
                    self.picture.set_from_pixbuf(GdkPixbuf.Pixbuf.new_from_bytes(data, GdkPixbuf.Colorspace.RGB,
                                                True, 8, width, height, width*4))
                    self.frames += count
            if ended or condition & GLib.IO_ERR:
                self.log.seek(0)
                raise ValueError(self.log.read()[-1200:].decode(errors='replace') or 'Action preview ended unexpectedly.')
        except (OSError, ValueError) as error:
            self.watch = None
            self.stop()
            self.failed(error)
            return False
        return True

    def stop(self):
        if self.closed:
            return
        self.closed = True
        if self.watch:
            GLib.source_remove(self.watch)
            self.watch = None
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=.5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        self.process.stdout.close()
        self.log.close()
