"""Bounded native message entrance/retirement motion, configured by the skin."""
import json
from dataclasses import replace
import time


def load_motion(assets):
    path = assets/'motion.json'
    if not path.exists():return None
    data = json.loads(path.read_text())
    limits = {'duration_ms':(100,1000),'fade_ms':(50,1000),'fade_delay_ms':(0,500),
              'enter_y':(0,80),'gap':(0,40),'max_history':(0,2)}
    if not isinstance(data, dict):raise ValueError('Invalid bubble motion')
    for key,(low,high) in limits.items():
        if type(data.get(key)) is not int or not low <= data[key] <= high:
            raise ValueError('Invalid bubble motion '+key)
    if data['fade_ms'] > data['duration_ms'] or data['fade_delay_ms'] >= data['duration_ms']:
        raise ValueError('Invalid bubble fade duration')
    return data


DEFAULT_MOTION = {'duration_ms': 480, 'fade_ms': 180, 'fade_delay_ms': 70,
                  'enter_y': 44, 'gap': 14, 'max_history': 2}


def smoothstep(value):
    value = max(0,min(1,value))
    return value*value*(3-2*value)


class WidgetFade:
    """A finite widget fade, reversible without a flash or a stale callback."""
    def __init__(self, widget, clock):
        self.widget, self.clock = widget, clock
        self.source = self.done = None
        self.target = widget.get_opacity()

    def to(self, target, duration, enabled=True, done=None):
        if self.source is not None and self.target == target:return
        self.stop()
        self.target, self.done = target, done
        self.initial = self.widget.get_opacity()
        self.duration, self.started = duration, None
        if not enabled or abs(self.initial-target)<.001:
            self.finish()
        else:self.source = self.clock.add_tick_callback(self.tick)

    def tick(self, *_):
        if self.started is None:self.started = time.monotonic()
        progress = min(1,(time.monotonic()-self.started)*1000/self.duration)
        self.widget.set_opacity(self.initial+(self.target-self.initial)*smoothstep(progress))
        if progress == 1:
            self.source = None
            self.finish()
            return False
        return True

    def finish(self):
        self.widget.set_opacity(self.target)
        done, self.done = self.done, None
        if done:done()

    def stop(self, finish=False):
        if self.source is not None:self.clock.remove_tick_callback(self.source)
        self.source = None
        if finish:self.finish()
        else:self.done = None


class BubbleFlow:
    def __init__(self, view, spec):
        self.view, self.spec = view, dict(spec or DEFAULT_MOTION)
        self.material_motion = spec is not None
        self.conversation = None
        self.history = []
        self.offset = 0
        self.started = None
        self.settings = view.window.get_settings()
        self.handler = self.settings.connect('notify::gtk-enable-animations', self.preference)
        view.window.connect('destroy', self.destroyed)

    def preference(self, *_):
        if not self.settings.get_property('gtk-enable-animations'):self.clear()

    @property
    def enabled(self):
        return self.material_motion or self.conversation is not None

    @staticmethod
    def channel(message):
        return (message.source, message.content.reply.key) if message and message.content.reply else None

    def retire(self, incoming):
        v = self.view
        channel = self.channel(incoming)
        old_channel = self.channel(v.message)
        if channel != old_channel:
            # History is presentation for this conversation, never another
            # plugin or another ChatGPT connection's transcript.
            self.clear()
        self.conversation = channel
        if (channel != old_channel or not self.enabled or not v.window.get_visible() or not v.message
                or not self.spec['max_history'] or not self.settings.get_property('gtk-enable-animations')):
            return
        old = type(v)(v.assets, lambda *_: True, lambda *_: None, lambda: None, transient=True)
        message = v.message
        if channel:
            message = replace(message, content=replace(message.content, reply=None, actions=(), status=''))
        old.render(message)
        old.window.set_accept_focus(False)
        if not channel:
            old.window.resize(*v.window.get_size())
        position = v.window.get_position()
        old.window.move(*position)
        old.show()
        old.surface.set_opacity(v.surface.get_opacity())
        self.history.append({'view': old, 'position': position, 'opacity': v.surface.get_opacity(),
                             'keep': bool(channel and message.content.speaker == 'user')})
        # One extra, noninteractive card may finish fading during a burst.
        limit = self.spec['max_history'] + bool(channel)
        while len(self.history) > limit:
            index = next((i for i, item in enumerate(self.history) if not item['keep']), 0)
            self.history.pop(index)['view'].destroy()

    def enter(self):
        if not self.settings.get_property('gtk-enable-animations'):
            self.clear();return
        for item in self.history:
            item['position'] = item['view'].window.get_position()
            item['opacity'] = item['view'].surface.get_opacity()
        if self.conversation:
            # Keep at most two user turns including the current one.
            budget = self.spec['max_history'] - (self.view.message.content.speaker == 'user')
            for item in reversed(self.history):
                if item['keep']:
                    if budget > 0:budget -= 1
                    else:item['keep'] = False
        self.started = time.monotonic()
        self.offset = self.spec['enter_y']
        self.view.surface.set_opacity(0)
        if self.view.fade is None:
            self.view.fade = self.view.window.add_tick_callback(self.tick)

    def tick(self, *_):
        elapsed = (time.monotonic()-self.started)*1000
        p = min(1,elapsed/self.spec['duration_ms']);ease = 1-(1-p)**3
        alpha = min(1,elapsed/self.spec['fade_ms'])
        self.view.surface.set_opacity(alpha*alpha*(3-2*alpha))
        self.offset = round(self.spec['enter_y']*(1-ease))
        self.view.move_to_position()
        if self.conversation:
            self.move_history(ease)
            if p == 1:
                self.view.fade = None
                self.finish()
                return False
            return True
        push = self.view.window.get_size()[1]+self.spec['gap']
        fade = max(0,min(1,(elapsed-self.spec['fade_delay_ms'])/(self.spec['duration_ms']-self.spec['fade_delay_ms'])))
        for item in self.history:
            x,y = item['position']
            item['view'].window.move(x,round(y-push*ease))
            item['view'].surface.set_opacity(item['opacity']*(1-fade*fade*(3-2*fade)))
        if p == 1:
            self.view.fade = None
            self.finish()
            return False
        return True

    def place_history(self, item, x, y, opacity):
        old = item['view']
        x, y = round(x), round(y)
        width, height = old.window.get_size()
        # A retained card must fit its own monitor. When space is insufficient,
        # hide the oldest overflow instead of drawing on another monitor. This
        # needs no Cairo foreign binding or animated X11 input-shape updates.
        work = self.view.workarea
        edge_alpha = 1
        if work:
            wx, wy, ww, wh = work
            edge_alpha = int(wx <= x and wy <= y and x+width <= wx+ww and y+height <= wy+wh)
        if old.window.get_position() != (x, y):old.window.move(x, y)
        old.surface.set_opacity(opacity*edge_alpha)

    def move_history(self, progress=None):
        if not self.view.position:return
        bx, by = self.view.position
        cursor, depth = by, 0
        for item in reversed(self.history):
            old = item['view']
            if item['keep']:
                cursor -= old.window.get_size()[1]+self.spec['gap']
                target_x, target_y = bx, cursor
                opacity = .76 if depth == 0 else .44
                depth += 1
            else:
                x, y = item['position']
                target_x, target_y = x, y-max(64, self.spec['enter_y'])
                opacity = 0
            if progress is not None:
                x, y = item['position']
                target_x, target_y = x+(target_x-x)*progress, y+(target_y-y)*progress
                opacity = item['opacity']+(opacity-item['opacity'])*progress
            self.place_history(item, target_x, target_y, opacity)

    def follow(self):
        if self.conversation and self.history and self.started is None:
            self.move_history()

    def reserve_height(self):
        if not self.conversation:return 0
        return sum(item['view'].window.get_size()[1]+self.spec['gap'] for item in self.history if item['keep'])

    def finish(self, preserve=True):
        self.started = None
        self.offset = 0
        self.view.surface.set_opacity(1)
        retained = []
        for item in self.history:
            if preserve and self.conversation and item['keep']:
                retained.append(item)
            else:
                item['view'].destroy()
        self.history = retained
        self.view.move_to_position()
        self.follow()

    def clear(self):
        if self.view.fade is not None:
            self.view.window.remove_tick_callback(self.view.fade)
            self.view.fade = None
        self.finish(preserve=False)

    def inspect(self):
        return [{'text': item['view'].message.content.text,
                 'speaker': item['view'].message.content.speaker,
                 'position': list(item['view'].window.get_position()),
                 'size': list(item['view'].window.get_size()),
                 'opacity': item['view'].surface.get_opacity(), 'retained': item['keep'],
                 'input_passthrough': item['view'].window.get_window().get_pass_through(),
                 'reply_visible': item['view'].composer.get_visible(),
                 'close_visible': item['view'].close.get_visible(),
                 'actions': len(item['view'].buttons),
                 'loading_timer': item['view'].loading.source is not None}
                for item in self.history]

    def destroyed(self, *_):
        if not self.view.disposed:self.clear()
        self.settings.disconnect(self.handler)
