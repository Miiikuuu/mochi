"""Generic message view. Renders content/actions without knowing their producer."""
import json
import time

import gi
gi.require_version('Gtk', '3.0')
gi.require_version('Gdk', '3.0')
from gi.repository import Gdk, Gtk, Pango
from bubble_layout import speech_place
from bubble_loading import LoadingAnimation
from bubble_decorations import decorate, load_decorations
from bubble_motion import BubbleFlow, load_motion
from messages import MessageState
from reply_composer import ReplyComposer


class SpeechBubble:
    def __init__(self, assets, close, action, resized, transient=False, reply=None):
        self.assets, self.transient = assets, transient
        self.disposed = False
        self.layout = json.loads((assets/'layout.json').read_text())
        if not isinstance(self.layout, dict):raise ValueError('Invalid bubble layout')
        load_decorations(assets)
        motion = load_motion(assets)
        limits = {'width': (220, 600), 'text_min_height': (24, 100),
                  'text_max_height': (100, 400), 'tail_size': (8, 24), 'gap': (0, 40)}
        for key, (low, high) in limits.items():
            value = self.layout.get(key)
            if type(value) is not int or not low <= value <= high:
                raise ValueError('Invalid bubble layout: '+key)
        if type(self.layout.get('inner_border',False)) is not bool:
            raise ValueError('Invalid inner border flag')
        if type(self.layout.get('notice_width',320)) is not int or not 220 <= self.layout.get('notice_width',320) <= 600:
            raise ValueError('Invalid notice width')
        if self.layout['text_max_height'] < self.layout['text_min_height']:
            raise ValueError('Invalid text height bounds')
        self.style = Gtk.CssProvider()
        self.style.load_from_path(str(assets/'speech.css'))
        self.buffer = Gtk.TextBuffer()
        self.message_id = None
        self.message = None
        self.actions_key = None
        self.buttons = {}
        self.on_action = action
        self.position = None
        self.workarea = None
        self.tail_side = 'bottom'
        self.tail_offset = 0
        self.tail_geometry = None
        self.fade = None
        self.text_height = self.layout['text_min_height']
        self.content_width = self.layout['width']
        self.window = Gtk.Window(title='Mochi · 角色气泡')
        self.window.set_wmclass('mochi-speech', 'MochiSpeech')
        self.window.set_decorated(False)
        self.window.set_resizable(False)
        self.window.set_keep_above(True)
        self.window.set_skip_taskbar_hint(True)
        self.window.set_skip_pager_hint(True)
        self.window.set_focus_on_map(False)
        self.window.set_type_hint(Gdk.WindowTypeHint.UTILITY)
        self.window.get_style_context().add_class('mochi-speech')
        visual = self.window.get_screen().get_rgba_visual()
        if visual:
            self.window.set_visual(visual)
        self.window.connect('delete-event', lambda *_: close(self.message_id))
        self.window.connect('key-press-event', lambda _w, e: close(self.message_id) if e.keyval == Gdk.KEY_Escape else False)
        self.window.connect('size-allocate', lambda *_: resized())
        grid = Gtk.Grid()
        self.surface = Gtk.Overlay()
        self.surface.add(grid)
        self.window.add(self.surface)
        self.card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        self.card.set_size_request(self.layout['width'], -1)
        self.card.get_style_context().add_class('mochi-speech-card')
        grid.attach(self.card, 1, 1, 1, 1)
        self.body = self.card
        if self.layout.get('inner_border'):
            self.body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
            self.body.get_style_context().add_class('mochi-speech-inner')
            self.card.pack_start(self.body, True, True, 0)
        self.tails = {}
        for side, column, row in [('top', 1, 0), ('bottom', 1, 2), ('left', 0, 1), ('right', 2, 1)]:
            horizontal = side in ('top', 'bottom')
            strip = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL if horizontal else Gtk.Orientation.VERTICAL)
            strip.set_size_request(-1 if horizontal else self.layout['tail_size'],
                                   self.layout['tail_size'] if horizontal else -1)
            arrow = Gtk.Box()
            arrow.set_size_request(*( (28, self.layout['tail_size']) if horizontal else (self.layout['tail_size'], 28) ))
            arrow.get_style_context().add_class('mochi-tail')
            arrow.get_style_context().add_class('mochi-tail-'+side)
            arrow.set_opacity(0)
            strip.pack_start(arrow, False, False, 0)
            grid.attach(strip, column, row, 1, 1)
            self.tails[side] = arrow
        header = Gtk.Box(spacing=8)
        self.heading = Gtk.Label(xalign=0)
        self.heading.set_ellipsize(Pango.EllipsizeMode.END)
        self.heading.get_style_context().add_class('mochi-speech-heading')
        header.pack_start(self.heading, True, True, 0)
        self.header_image = decorate(assets, self.surface, header)
        self.close = Gtk.Button(label='×')
        self.close.set_valign(Gtk.Align.CENTER)
        self.close.set_tooltip_text('关闭消息')
        self.close.get_style_context().add_class('mochi-speech-close')
        if transient:self.close.set_no_show_all(True)
        self.close.connect('clicked', lambda *_: close(self.message_id))
        header.pack_end(self.close, False, False, 0)
        self.body.pack_start(header, False, False, 0)
        self.status = Gtk.Label(xalign=0)
        self.status.set_line_wrap(True)
        self.status.set_max_width_chars(28)
        self.status.get_style_context().add_class('mochi-speech-status')
        self.loading = LoadingAnimation(assets, static=transient)
        self.status_row = Gtk.Box(spacing=self.loading.gap)
        if self.loading.placement == 'status':
            self.status_row.pack_start(self.loading, False, False, 0)
        self.status_row.pack_start(self.status, True, True, 0)
        self.body.pack_start(self.status_row, False, False, 0)
        self.text = Gtk.TextView(buffer=self.buffer, editable=False, cursor_visible=False,
                                 wrap_mode=Gtk.WrapMode.WORD_CHAR)
        self.text.get_style_context().add_class('mochi-speech-text')
        self.text.set_top_margin(2)
        self.text.set_bottom_margin(2)
        self.scroll = Gtk.ScrolledWindow()
        self.scroll.set_size_request(-1, self.text_height)
        self.scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.scroll.add(self.text)
        self.body.pack_start(self.scroll, True, True, 0)
        self.composer = ReplyComposer(lambda text: reply(self.message_id, text) if reply else None, resized)
        self.body.pack_start(self.composer, False, False, 0)
        if self.loading.placement == 'footer':
            self.loading.set_halign(Gtk.Align.START)
            self.body.pack_start(self.loading, False, False, 0)
        self.footer = Gtk.FlowBox()
        self.footer.set_selection_mode(Gtk.SelectionMode.NONE)
        self.footer.set_min_children_per_line(1)
        self.footer.set_max_children_per_line(3)
        self.footer.set_column_spacing(4)
        self.footer.set_row_spacing(4)
        self.body.pack_end(self.footer, False, False, 0)
        for widget in [self.status_row, self.status, self.scroll, self.footer]:
            widget.set_no_show_all(True)
        self.buffer_handler = self.buffer.connect('changed', lambda *_: self.measure())
        self.flow = BubbleFlow(self, motion) if not transient else None
        self.apply_style(self.window)

    def apply_style(self, widget):
        # Per-widget providers prevent one DIY theme leaking into another or
        # into the translation management panel during a live switch.
        widget.get_style_context().add_provider(self.style, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        if isinstance(widget, Gtk.Container):
            for child in widget.get_children():self.apply_style(child)

    def measure(self):
        if self.disposed:return
        text = self.buffer.get_text(self.buffer.get_start_iter(), self.buffer.get_end_iter(), True)
        layout = self.text.create_pango_layout(text)
        layout.set_width((self.content_width-36)*Pango.SCALE)
        layout.set_wrap(Pango.WrapMode.WORD_CHAR)
        height = max(self.layout['text_min_height'], min(layout.get_pixel_size()[1]+8, self.layout['text_max_height']))
        if height != self.text_height:
            self.text_height = height
            self.scroll.set_size_request(-1, height)
            self.window.resize(1, 1)

    def render(self, message):
        content = message.content
        new_message = message.id != self.message_id
        if new_message and self.flow:self.flow.retire(message)
        self.message_id = message.id
        self.message = message
        for kind in ('chat','choice','notice'):
            context = self.card.get_style_context()
            if kind == content.kind:context.add_class('mochi-kind-'+kind)
            else:context.remove_class('mochi-kind-'+kind)
        width = self.layout.get('notice_width', self.layout['width']) if content.kind == 'notice' else self.layout['width']
        self.content_width = width
        self.card.set_size_request(width, -1)
        if self.header_image:self.header_image.set_visible(content.kind != 'notice')
        old_text = self.buffer.get_text(self.buffer.get_start_iter(), self.buffer.get_end_iter(), True)
        if old_text != content.text:
            self.buffer.set_text(content.text)
        self.heading.set_text(content.title)
        self.status.set_text(content.status[:240])
        self.status.set_visible(bool(content.status) and content.show_status)
        self.loading.set_loading(message.state == MessageState.RUNNING, restart=new_message)
        if content.kind == 'notice' and not self.loading.loading and self.loading.placement == 'footer':
            self.loading.hide()
        self.status_row.set_visible((self.loading.placement == 'status' and self.loading.get_visible()) or self.status.get_visible())
        self.scroll.set_visible(bool(content.text))
        self.composer.render(content.reply if not self.transient else None)
        key = (message.id, content.actions)
        if key != self.actions_key:
            self.footer.set_max_children_per_line(min(3, max(1, len(content.actions))))
            for child in self.footer.get_children():
                child.destroy()
            self.buttons = {}
            for index, item in enumerate(content.actions):
                button = Gtk.Button(label=item.label)
                button.set_sensitive(item.enabled)
                if index == 0:button.get_style_context().add_class('mochi-first-action')
                label = button.get_child()
                label.set_ellipsize(Pango.EllipsizeMode.END)
                label.set_max_width_chars(10)
                button.set_tooltip_text(item.tooltip or item.label)
                button.connect('clicked', lambda _b, mid=message.id, key=item.id: self.on_action(mid, key))
                self.footer.add(button)
                button.get_parent().set_halign(Gtk.Align.END if item.align == 'end' else Gtk.Align.START)
                self.buttons[item.id] = button
            # no_show_all on the footer protects its state; show new children
            # explicitly so actions are realized even when the bubble is hidden.
            for child in self.footer.get_children():
                self.apply_style(child)
                child.show_all()
            self.actions_key = key
            self.window.resize(1, 1)
        self.footer.set_visible(bool(content.actions))
        self.measure()
        if new_message and self.flow and self.flow.enabled and self.window.get_visible():self.flow.enter()

    def show(self):
        if self.window.get_visible():
            return
        if self.transient:
            self.window.realize()
            self.window.get_window().set_pass_through(True)
            self.surface.set_opacity(1)
            self.window.show_all()
            self.text.show()
            return
        self.surface.set_opacity(0)
        self.window.show_all()
        # no_show_all on the scroller preserves its state but its text child
        # still needs to be realized for selection and scrolling.
        self.text.show()
        self.window.realize()
        self.position = None
        if self.flow and self.flow.enabled:
            self.flow.enter()
            return
        if not self.window.get_settings().get_property('gtk-enable-animations'):
            self.surface.set_opacity(1)
            return
        started = time.monotonic()
        def tick(_widget, _clock):
            p = min(1, (time.monotonic()-started)/0.16)
            self.surface.set_opacity(1-(1-p)**3)
            if p == 1:
                self.fade = None
                return False
            return True
        self.fade = self.window.add_tick_callback(tick)

    def hide(self):
        self.loading.stop()
        if self.flow:self.flow.clear()
        if self.fade is not None:
            self.window.remove_tick_callback(self.fade)
            self.fade = None
        self.window.hide()

    def clear(self):
        self.hide()
        self.loading.set_loading(False)
        self.message_id = self.actions_key = None
        self.message = None
        self.buffer.set_text('')
        self.heading.set_text('')
        self.status.set_text('')
        self.status_row.hide()
        for child in self.footer.get_children():
            child.destroy()
        self.buttons = {}
        self.footer.hide()
        self.scroll.hide()
        self.composer.render(None)

    def destroy(self):
        if self.disposed:return
        self.hide()
        self.disposed = True
        # Detach the scroll adjustments before destroying the text view. A
        # ScrolledWindow tearing down around an already-destroyed TextView can
        # otherwise notify it after GTK has released its buffer/layout marks.
        self.buffer.disconnect(self.buffer_handler)
        self.scroll.remove(self.text)
        self.text.destroy()
        self.buffer.set_text('')
        self.message = None
        self.window.destroy()

    def move_to_position(self):
        if self.position and self.window.get_realized():
            x,y = self.position
            y += self.flow.offset if self.flow else 0
            if self.workarea:
                wx,wy,ww,wh = self.workarea
                y = max(wy,min(y,wy+max(0,wh-self.window.get_size()[1])))
            self.window.move(x,y)

    def follow(self, anchor, work, left_limit=None):
        self.workarea = work
        width, height = self.window.get_size()
        position, side, offset = speech_place(anchor, (width, height), work, self.layout['gap'], self.layout['tail_size'], left_limit)
        reserve = self.flow.reserve_height() if self.flow else 0
        if reserve:
            # A pet near the upper edge still gets a readable conversation
            # column: make room for retained user turns above the current one.
            x, y = position
            wx, wy, ww, wh = work
            y = max(y, min(wy+reserve, wy+max(0, wh-height)))
            position = x, y
            ax, ay, aw, ah = anchor
            if y >= ay+ah:side = 'top'
            elif y+height <= ay:side = 'bottom'
            elif x >= ax+aw:side = 'left'
            elif x+width <= ax:side = 'right'
            point, length = ((ax+aw/2-x, width) if side in ('top', 'bottom') else (ay+ah/2-y, height))
            inset = self.layout['tail_size']+26
            offset = round(max(inset, min(point, max(inset, length-inset))))
        if position != self.position:
            self.position = position
            self.move_to_position()
        tail = self.layout['tail_size']
        geometry = (side, offset, width, height)
        if geometry != self.tail_geometry:
            for key, arrow in self.tails.items():
                arrow.set_opacity(1 if key == side else 0)
            if side in ('top', 'bottom'):
                self.tails[side].set_margin_start(max(0, offset-tail-14))
            else:
                self.tails[side].set_margin_top(max(0, offset-tail-14))
            self.tail_geometry = geometry
        self.tail_side, self.tail_offset = side, offset
        if self.flow:self.flow.follow()

    def inspect(self):
        controls = {}
        for name, widget in {**self.buttons, 'close': self.close, 'reply_input': self.composer.entry, 'reply_send': self.composer.button}.items():
            xy = widget.translate_coordinates(self.window, 0, 0)
            rect = widget.get_allocation()
            controls[name] = [*xy, rect.width, rect.height] if xy else None
        card_xy = self.card.translate_coordinates(self.window, 0, 0)
        card_size = self.card.get_allocation()
        arrow = self.tails[self.tail_side]
        tail_xy = arrow.translate_coordinates(self.window, 0, 0)
        tail_size = arrow.get_allocation()
        return {'message_id': self.message_id, 'title': self.heading.get_text(), 'status': self.status.get_text(),
                'reply_visible': self.composer.get_visible(), 'reply_text': self.composer.entry.get_text(),
                'reply_enabled': self.composer.button.get_sensitive(),
                'text': self.buffer.get_text(self.buffer.get_start_iter(), self.buffer.get_end_iter(), True),
                'actions': {key: button.get_label() for key, button in self.buttons.items()},
                'visible': self.window.get_visible(), 'position': self.position,
                'size': list(self.window.get_size()), 'tail': self.tail_side,
                'tail_offset': self.tail_offset, 'text_height': self.text_height,
                'loading': self.loading.inspect(),
                'theme': self.assets.name, 'kind': self.message.content.kind if self.message else None,
                'header_image_visible': bool(self.header_image and self.header_image.get_visible()),
                'text_bounds': [*self.text.translate_coordinates(self.window,0,0), self.text.get_allocated_width(), self.text.get_allocated_height()] if self.text.get_mapped() else None,
                'retiring': len(self.flow.history) if self.flow else 0,
                'history': self.flow.inspect() if self.flow else [],
                'speaker': self.message.content.speaker if self.message else '',
                'animating': self.fade is not None, 'controls': controls,
                'card_bounds': [*card_xy, card_size.width, card_size.height] if card_xy else None,
                'tail_bounds': [*tail_xy, tail_size.width, tail_size.height] if tail_xy else None,
                'tail_geometry': self.tail_geometry,
                'window': self.window.get_window().get_xid() if self.window.get_realized() else None}
