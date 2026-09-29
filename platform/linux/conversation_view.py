"""Virtualized native conversation: separate message cards and a fixed composer."""
import json
import time

import gi
gi.require_version('Gtk', '3.0')
gi.require_version('Gdk', '3.0')
from gi.repository import Gdk, GLib, Gtk, Pango
from bubble_decorations import decorate
from bubble_controls import apply_controls
from bubble_layout import place, tail_attachment
from bubble_loading import LoadingAnimation
from bubble_motion import DEFAULT_MOTION, WidgetFade, load_motion, smoothstep
from reply_composer import ReplyComposer


def materials(assets):
    path = assets/'conversation.json'
    spec = json.loads(path.read_text()) if path.exists() else {'version':1}
    if not isinstance(spec, dict) or spec.get('version') != 1:
        raise ValueError('Invalid conversation material')
    height = spec.get('viewport_height', 224)
    if type(height) is not int or not 160 <= height <= 480:
        raise ValueError('Invalid conversation viewport height')
    result = {}
    for role in ('assistant', 'user', 'composer'):
        name = spec.get(role, '.')
        if not isinstance(name, str):raise ValueError('Invalid conversation material path')
        folder = (assets/name).resolve()
        if not folder.is_relative_to(assets.resolve()) or not (folder/'speech.css').is_file():
            raise ValueError('Conversation material must stay inside its theme')
        result[role] = folder
    width = spec.get('width', 304)
    if type(width) is not int or not 240 <= width <= 420:
        raise ValueError('Invalid conversation width')
    return result, height, width


def style_tree(widget, provider):
    widget.get_style_context().add_provider(provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
    if isinstance(widget, Gtk.Container):
        for child in widget.get_children():style_tree(child, provider)


class Paper(Gtk.Overlay):
    """One material instance; never contains another turn or a copied input."""
    tail_depth = 12
    tail_span = 24

    def __init__(self, assets, role, width, composer=None):
        super().__init__()
        self.role, self.assets = role, assets
        self.get_style_context().add_class('mochi-compact-paper')
        self.style = Gtk.CssProvider()
        self.style.load_from_path(str(assets/'speech.css'))
        self.set_size_request(width, -1)
        grid = Gtk.Grid()
        grid.set_margin_bottom(4)
        self.add(grid)
        self.card = card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.waiting_allocation = True
        card.connect('size-allocate', lambda *_: setattr(self,'waiting_allocation',False))
        card.set_hexpand(True)
        card.get_style_context().add_class('mochi-speech-card')
        body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        body.get_style_context().add_class('mochi-speech-inner')
        card.add(body)
        grid.attach(card, 1, 1, 1, 1)
        self.tails = {}
        self.tail_geometry = self.tail_state = None
        # Reserve every edge once. Turning a tail must never change wrapping,
        # card heights, scroll offsets, or the input window's hit region.
        for side,column,row in [('top',1,0),('bottom',1,2),('left',0,1),('right',2,1)]:
            horizontal = side in ('top','bottom')
            strip = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL if horizontal else Gtk.Orientation.VERTICAL)
            strip.set_size_request(-1 if horizontal else self.tail_depth, self.tail_depth if horizontal else -1)
            tail = Gtk.Box()
            tail.set_size_request(*((self.tail_span,self.tail_depth) if horizontal else (self.tail_depth,self.tail_span)))
            tail.get_style_context().add_class('mochi-tail')
            tail.get_style_context().add_class('mochi-tail-'+side)
            tail.set_opacity(0)
            strip.pack_start(tail,False,False,0)
            grid.attach(strip,column,row,1,1)
            self.tails[side] = tail
        self.header = Gtk.Box(spacing=8)
        self.title = Gtk.Label(xalign=0)
        self.title.set_ellipsize(Pango.EllipsizeMode.END)
        self.title.get_style_context().add_class('mochi-speech-heading')
        self.header.pack_start(self.title, True, True, 0)
        header_image = decorate(assets, self, self.header, scale=.75)
        if header_image:header_image.show()
        body.pack_start(self.header, False, False, 0)
        self.text = None
        if composer:
            self.header.set_no_show_all(True)
            self.header.hide()
            body.pack_start(composer, False, False, 0)
        else:
            self.text = Gtk.Label(xalign=0, yalign=0, selectable=True)
            self.text.set_line_wrap(True)
            self.text.set_line_wrap_mode(Pango.WrapMode.WORD_CHAR)
            self.text.set_width_chars(1)
            self.text.set_max_width_chars(1)
            self.text.get_style_context().add_class('mochi-speech-text')
            body.pack_start(self.text, False, False, 0)
        style_tree(self, self.style)
        self.compact_style = Gtk.CssProvider()
        self.compact_style.load_from_data(b'.mochi-compact-paper .mochi-speech-card {padding: 3px;} .mochi-compact-paper .mochi-speech-inner {padding: 6px;} .mochi-compact-paper .mochi-speech-text, .mochi-compact-paper .mochi-speech-text text {font-size: 14px;} .mochi-compact-paper button {padding: 3px 6px;}')
        style_tree(self, self.compact_style)
        self.show_all()

    def render(self, turn):
        self.title.set_text(turn.title)
        self.text.set_text(turn.text)
        # A shortened update must not retain the old arrow's margin as a
        # minimum size. Re-aim after GTK has allocated the new text height.
        self.waiting_allocation = True
        self.tail_geometry = self.tail_state = None
        for tail in self.tails.values():
            tail.set_margin_start(0)
            tail.set_margin_top(0)
            tail.set_opacity(0)

    def point_to(self, anchor, origin):
        if self.waiting_allocation:return
        xy = self.card.translate_coordinates(self,0,0)
        width, height = self.card.get_allocated_width(), self.card.get_allocated_height()
        if not xy or min(width,height) <= 1:return
        card = (origin[0]+xy[0],origin[1]+xy[1],width,height)
        attachment = tail_attachment(anchor,card)
        self.tail_state = {'card':list(card), 'target':[anchor[0]+anchor[2]/2,anchor[1]+anchor[3]/2],
                           'side':attachment[0] if attachment else None, 'offset':attachment[1] if attachment else None}
        if attachment == self.tail_geometry:return
        self.tail_geometry = attachment
        for side,tail in self.tails.items():
            tail.set_opacity(1 if attachment and side == attachment[0] else 0)
        if attachment:
            side,offset = attachment
            tail = self.tails[side]
            if side in ('top','bottom'):tail.set_margin_start(max(0,offset-self.tail_span//2))
            else:tail.set_margin_top(max(0,offset-self.tail_span//2))
            # Gtk.Layout can skip an unchanged paper requisition. Reallocate
            # just this edge after the parent allocation, without restoring an
            # old paper rectangle over a pending message animation position.
            strip = tail.get_parent()
            strip.queue_allocate()
            strip.size_allocate(strip.get_allocation())



class ConversationView:
    max_widgets = 6

    def __init__(self, assets, close, action, resized, reply, history):
        self.assets, self.history = assets, history
        self.paths, self.preferred_height, self.width = materials(assets)
        self.resized = resized
        self.editing = False
        self.composer_expanded = self.closing = False
        self.resize_source = self.focus_source = self.blur_source = self.tail_source = None
        self.height_limit = self.preferred_height
        self.motion = load_motion(assets) or DEFAULT_MOTION
        self.message = None
        self.message_id = None
        self.position = None
        self.anchor = None
        self.disposed = False
        self.fade = None
        self.started = None
        self.push = 0
        self.new_id = None
        self.seen_version = -1
        self.channel = None
        self.rows = []
        self.widgets = {}
        self.heights = {}
        self.total = 0
        self.scrolled = False
        self.suppress_scroll = False
        self.viewport_height = self.preferred_height
        self.window = Gtk.Window(title='Mochi · 对话')
        self.window.set_wmclass('mochi-conversation', 'MochiConversation')
        self.window.set_decorated(False)
        self.window.set_resizable(False)
        self.window.set_keep_above(True)
        self.window.set_skip_taskbar_hint(True)
        self.window.set_skip_pager_hint(True)
        self.window.set_focus_on_map(False)
        self.window.set_type_hint(Gdk.WindowTypeHint.UTILITY)
        self.window.get_style_context().add_class('mochi-conversation')
        visual = self.window.get_screen().get_rgba_visual()
        if visual:self.window.set_visual(visual)
        self.window.connect('delete-event', lambda *_: close(self.message_id))
        self.window.connect('key-press-event', lambda _w,e: close(self.message_id) if e.keyval == Gdk.KEY_Escape else False)
        self.window.connect('size-allocate', lambda *_: resized())
        self.window.connect('scroll-event', self.wheel)
        self.settings = self.window.get_settings()
        self.preference_handler = self.settings.connect('notify::gtk-enable-animations', self.motion_preference)
        self.surface = root = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        self.window.add(root)
        self.visibility_fade = WidgetFade(root,self.window)
        self.layout = Gtk.Layout()
        self.scroll = Gtk.ScrolledWindow()
        # External policy keeps a real adjustment but no scrollbar/chrome.
        self.scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.EXTERNAL)
        self.scroll.set_size_request(self.width, self.viewport_height)
        self.scroll.add(self.layout)
        self.scroll.connect('scroll-event', self.wheel)
        root.pack_start(self.scroll, False, False, 0)
        self.adjustment = self.scroll.get_vadjustment()
        self.adjustment.connect('value-changed', self.scroll_changed)
        self.composer = ReplyComposer(lambda text: reply(self.message_id, text), resized, multiline=True)
        self.composer.scroll.set_size_request(-1, 34)
        self.composer_paper = Paper(self.paths['composer'], 'composer', self.width, self.composer)
        self.composer_paper.set_no_show_all(True)
        self.composer_paper.hide()
        root.pack_start(self.composer_paper, False, False, 0)
        self.composer_fade = WidgetFade(self.composer_paper,self.window)
        self.toolbar = Gtk.Box(spacing=4)
        self.toolbar.set_margin_start(12)
        self.toolbar.set_margin_end(12)
        self.toolbar.set_margin_bottom(4)
        self.toolbar.get_style_context().add_class('mochi-chat-tools')
        root.pack_start(self.toolbar, False, False, 0)
        self.close = Gtk.Button(label='×')
        self.close.get_style_context().add_class('mochi-speech-close')
        self.close.set_tooltip_text('关闭对话气泡')
        self.close.get_accessible().set_name('关闭对话气泡')
        self.close.connect('clicked', lambda *_: close(self.message_id))
        self.toolbar.pack_end(self.close, False, False, 0)
        self.open_reply = Gtk.Button.new_from_icon_name('document-edit-symbolic', Gtk.IconSize.MENU)
        self.open_reply.set_tooltip_text('回复')
        self.open_reply.get_accessible().set_name('回复')
        self.open_reply.set_no_show_all(True)
        self.open_reply.connect('clicked', lambda *_: self.begin_reply())
        self.toolbar.pack_end(self.open_reply, False, False, 0)
        self.latest = Gtk.Button.new_from_icon_name('go-bottom-symbolic', Gtk.IconSize.BUTTON)
        self.latest.set_tooltip_text('回到最新消息')
        self.latest.get_accessible().set_name('回到最新消息')
        self.latest.set_no_show_all(True)
        self.latest.connect('clicked', lambda *_: self.go_latest())
        self.toolbar.pack_end(self.latest, False, False, 0)
        self.loading = LoadingAnimation(self.paths['composer'])
        self.toolbar.pack_start(self.loading, False, False, 0)
        self.status = Gtk.Label(xalign=0)
        self.status.set_line_wrap(True)
        self.status.set_line_wrap_mode(Pango.WrapMode.WORD_CHAR)
        self.status.set_width_chars(1)
        self.status.set_max_width_chars(18)
        self.status.get_style_context().add_class('mochi-speech-status')
        self.status.set_no_show_all(True)
        self.toolbar.pack_start(self.status, True, True, 0)
        style_tree(self.toolbar, self.composer_paper.style)
        self.chrome = Gtk.CssProvider()
        self.chrome.load_from_data(b'window.mochi-conversation, .mochi-conversation scrolledwindow, .mochi-conversation layout {background: transparent;} .mochi-conversation scrolledwindow {border: none; box-shadow: none;} .mochi-conversation undershoot, .mochi-conversation overshoot {background: none; box-shadow: none;} .mochi-chat-tools button {padding: 0; min-width: 24px; min-height: 24px; border: 0; border-radius: 12px; background: alpha(#fffef8,.92); color: #5287a4; box-shadow: none;} .mochi-chat-tools button:hover {background: #eaf6f9;} .mochi-chat-tools .mochi-speech-status {font-size: 12px;}')
        style_tree(self.window, self.chrome)
        self.composer.button.get_accessible().set_name('发送回复')
        self.control_style = apply_controls(assets,self.window,{'reply':self.open_reply,'close':self.close,'latest':self.latest,'send':self.composer.button})
        # Two unparented measurement widgets, reused for every text record.
        self.probes = {role:Paper(self.paths[role], role, self.width-16) for role in ('user','assistant')}
        self.composer.entry.get_buffer().connect('changed', self.draft_changed)
        self.composer.entry.connect('preedit-changed', lambda *_: self.sync_composer())
        self.composer.entry.connect('focus-out-event', self.blur_reply)
        root.show_all()

    def request_resize(self):
        if self.resize_source is None and not self.disposed:
            def resize():
                self.resize_source = None
                self.window.resize(1,1)
                self.resized()
                return False
            self.resize_source = GLib.idle_add(resize)

    def sync_composer(self):
        if self.disposed:return
        expanded = bool(self.composer.spec and (self.editing or self.composer.entry.get_text() or self.composer.entry.preedit))
        changed = self.composer_expanded != expanded
        self.composer_expanded = expanded
        if not expanded:
            self.open_reply.set_visible(bool(self.composer.spec))
            if changed and self.composer.spec:self.window.set_focus(self.open_reply)
        if changed:
            self.composer_paper.set_sensitive(expanded)
            if expanded:
                if not self.composer_paper.get_visible():self.composer_paper.set_opacity(0)
                self.composer_paper.show()
                self.request_resize()
            def collapse():
                self.composer_paper.hide()
                self.request_resize()
            self.composer_fade.to(1 if expanded else 0,self.motion['fade_ms'],
                self.window.get_visible() and self.settings.get_property('gtk-enable-animations'),None if expanded else collapse)
        if expanded and self.open_reply.has_focus():self.window.set_focus(self.composer.entry)
        self.open_reply.set_visible(bool(self.composer.spec) and not expanded)
        # Keep only real loading/status feedback in the collapsed toolbar.
        self.loading.set_visible(self.loading.loading)

    def draft_changed(self, *_):
        entry, composer = self.composer.entry, self.composer
        if entry.get_text():self.editing = True
        elif (not entry.has_focus() or (composer.spec and
              (composer.spec.key != composer.key or (composer.submitted and composer.spec.clear_id == composer.submitted[0])))):
            self.editing = False
        self.sync_composer()

    def begin_reply(self):
        if not self.composer.spec:return
        self.editing = True
        self.sync_composer()
        if self.focus_source is not None:GLib.source_remove(self.focus_source)
        def focus():
            self.focus_source = None
            if not self.disposed and self.window.get_visible() and self.composer_paper.get_visible():
                self.composer.entry.grab_focus()
            return False
        self.focus_source = GLib.idle_add(focus)

    def blur_reply(self, *_):
        if self.blur_source is not None:GLib.source_remove(self.blur_source)
        def collapse():
            self.blur_source = None
            if not self.composer.entry.has_focus() and not self.composer.entry.get_text() and not self.composer.entry.preedit:
                self.editing = False
                self.sync_composer()
            return False
        self.blur_source = GLib.idle_add(collapse)
        return False

    def maximum(self):
        return max(0, self.total-self.viewport_height)

    def geometry(self):
        rows, cursor, alive = [], 0, set()
        for turn in self.history.turns:
            alive.add(turn.id)
            signature = (turn.speaker, turn.title, turn.text)
            cached = self.heights.get(turn.id)
            if not cached or cached[0] != signature:
                probe = self.probes[turn.speaker]
                probe.render(turn)
                height = probe.get_preferred_height_for_width(self.width-16)[1]
                self.heights[turn.id] = (signature, max(72, height))
            height = self.heights[turn.id][1]
            rows.append((turn, cursor, height))
            cursor += height+self.motion['gap']
        self.heights = {key:value for key,value in self.heights.items() if key in alive}
        height = min(self.height_limit, max(72,cursor))
        if height != self.viewport_height:
            self.viewport_height = height
            self.scroll.set_size_request(self.width,height)
            self.request_resize()
        self.total = max(self.viewport_height, cursor)
        # Short conversations sit at the bottom, directly above the input.
        inset = max(0, self.viewport_height-cursor)
        self.rows = [(turn,y+inset,h) for turn,y,h in rows]
        self.suppress_scroll = True
        self.layout.set_size(self.width, self.total)
        self.adjustment.configure(self.adjustment.get_value(), 0, self.total, 48, self.viewport_height*.8, self.viewport_height)
        self.suppress_scroll = False

    def render(self, message):
        self.message, self.message_id = message, message.id
        content = message.content
        changed_channel = self.channel != self.history.channel
        old_max = self.maximum()
        old_value = self.adjustment.get_value()
        old_rows = self.rows
        status_changed = self.status.get_text() != content.status or self.loading.loading != content.reply.waiting
        self.composer.render(content.reply)
        self.loading.set_loading(content.reply.waiting)
        self.sync_composer()
        self.status.set_text(content.status)
        self.status.set_visible(bool(content.status))
        if status_changed:self.request_resize()
        if self.seen_version == self.history.version:return
        self.stop_motion()
        anchor = next(((turn.id, old_value-y) for turn,y,h in old_rows if y+h > old_value), None)
        self.channel = self.history.channel
        self.seen_version = self.history.version
        self.geometry()
        if changed_channel:self.scrolled = False
        target = self.maximum()
        if self.scrolled and anchor:
            target = next((y+anchor[1] for turn,y,h in self.rows if turn.id == anchor[0]), 0)
        self.set_scroll(target)
        self.new_id = message.id
        self.push = max(0, self.maximum()-old_max)
        if not self.scrolled and not changed_channel and self.window.get_visible() and self.settings.get_property('gtk-enable-animations'):
            # Start on the first frame, after new GTK/Pango/material widgets
            # have been allocated; cold image decoding must not eat the move.
            self.started = None
            self.fade = self.window.add_tick_callback(self.tick)
            self.paint(0)
        else:self.paint()

    def set_scroll(self, value):
        self.suppress_scroll = True
        self.adjustment.set_value(max(0, min(value, self.maximum())))
        self.suppress_scroll = False
        self.latest.set_visible(self.scrolled)

    def wheel(self, _widget, event):
        delta = -64 if event.direction == Gdk.ScrollDirection.UP else 64 if event.direction == Gdk.ScrollDirection.DOWN else 0
        if event.direction == Gdk.ScrollDirection.SMOOTH:
            valid, _dx, dy = event.get_scroll_deltas()
            if valid:delta = dy*48
        if not delta:return False
        # The input's own text scroller handles its wheel first. Here we only
        # browse the message region; messages cannot take focus from a draft.
        target = max(0, min(self.maximum(), self.adjustment.get_value()+delta))
        # Boundary wheel events are common while the pointer rests on a bubble.
        # They must not cancel an incoming card's animation when nothing scrolls.
        if target != self.adjustment.get_value():
            self.stop_motion()
            self.adjustment.set_value(target)
        return True

    def scroll_changed(self, *_):
        if self.suppress_scroll or self.disposed:return
        self.scrolled = self.adjustment.get_value() < self.maximum()-2
        self.latest.set_visible(self.scrolled)
        self.paint()

    def go_latest(self):
        self.stop_motion()
        self.scrolled = False
        self.set_scroll(self.maximum())
        self.paint()

    def paint(self, progress=1):
        if self.disposed:return
        top = self.adjustment.get_value()
        ease = 1-(1-progress)**3
        wanted = []
        for turn, y, height in self.rows:
            offset = (self.motion['enter_y'] if turn.id == self.new_id else self.push)*(1-ease)
            visual_y = y+offset
            if visual_y+height >= top and visual_y < top+self.viewport_height:
                wanted.append((turn, visual_y, height, y))
        wanted = wanted[-self.max_widgets:]
        ids = {turn.id for turn,_,_,_ in wanted}
        for identity in list(self.widgets):
            if identity not in ids:self.widgets.pop(identity).destroy()
        for turn,y,height,final_y in wanted:
            widget = self.widgets.get(turn.id)
            if widget is None:
                widget = Paper(self.paths[turn.speaker], turn.speaker, self.width-16)
                widget.render(turn)
                self.widgets[turn.id] = widget
                self.layout.put(widget, 16 if turn.speaker == 'user' else 0, round(y))
            else:
                if widget.text.get_text() != turn.text or widget.title.get_text() != turn.title:widget.render(turn)
                self.layout.move(widget, 16 if turn.speaker == 'user' else 0, round(y))
            # Whole cards fade as their trailing edge leaves the upper edge.
            # In scrollback keep text fully legible, including tall messages.
            alpha = 1 if self.scrolled or height > self.viewport_height else smoothstep((y+height-top)/max(80, height))
            elapsed = progress*self.motion['duration_ms']
            if turn.id == self.new_id and progress < 1:alpha *= smoothstep(elapsed/self.motion['fade_ms'])
            elif not self.scrolled and progress < 1:
                if final_y+height <= top:
                    # Fade an outgoing card while it is still visible, rather
                    # than relying only on clipping at the top of the scroller.
                    alpha *= 1-smoothstep((elapsed-self.motion['fade_delay_ms'])/self.motion['fade_ms'])
            widget.set_opacity(alpha)
        self.point_tails()

    def point_tails(self):
        if self.tail_source is None and not self.disposed:
            self.tail_source = GLib.idle_add(self.aim_tails)

    def aim_tails(self):
        self.tail_source = None
        if not self.position or not self.anchor:return
        # Follow can run during a parent size allocation. Defer tail changes
        # until that finishes, using allocated coordinates including scrolling.
        papers = list(self.widgets.values())
        if self.composer_paper.get_visible():papers.append(self.composer_paper)
        for paper in papers:
            xy = paper.translate_coordinates(self.window,0,0)
            if xy:paper.point_to(self.anchor,(self.position[0]+xy[0],self.position[1]+xy[1]))
        return False

    def tick(self, *_):
        if self.started is None:self.started = time.monotonic()
        progress = min(1, (time.monotonic()-self.started)*1000/self.motion['duration_ms'])
        self.paint(progress)
        if progress == 1:
            self.fade = None
            self.started = None
            return False
        return True

    def stop_motion(self):
        if self.fade is not None:self.window.remove_tick_callback(self.fade)
        self.fade = self.started = None
        self.push = 0

    def motion_preference(self, *_):
        if not self.settings.get_property('gtk-enable-animations'):
            self.visibility_fade.stop(finish=True)
            self.composer_fade.stop(finish=True)
            self.stop_motion()
            if self.window.get_visible():self.paint()

    def show(self):
        if self.window.get_visible() and not self.closing:return
        was_visible = self.window.get_visible()
        self.closing = False
        self.surface.set_sensitive(True)
        if not was_visible:self.surface.set_opacity(0)
        if not self.composer_paper.get_visible():self.window.set_focus(self.open_reply)
        self.window.show()
        self.visibility_fade.to(1,self.motion['fade_ms'],self.settings.get_property('gtk-enable-animations'))
        self.loading.sync()
        self.paint()

    def hide(self):
        self.visibility_fade.stop()
        self.composer_fade.stop(finish=True)
        self.closing = False
        self.stop_motion()
        self.loading.stop()
        self.window.hide()
        if not self.composer.entry.get_text():self.editing = False
        self.sync_composer()
        for name in ('resize_source','focus_source','blur_source','tail_source'):
            if getattr(self,name) is not None:GLib.source_remove(getattr(self,name))
            setattr(self,name,None)
        for widget in self.widgets.values():widget.destroy()
        self.widgets.clear()

    def clear(self):
        self.message = None
        self.message_id = None
        self.stop_motion()
        self.loading.stop()
        self.surface.set_sensitive(False)
        self.closing = True
        def cleared():
            self.hide()
            self.composer.render(None)
        self.visibility_fade.to(0,self.motion['fade_ms'],self.window.get_visible() and self.settings.get_property('gtk-enable-animations'),cleared)

    def follow(self, anchor, work, left_limit=None):
        self.anchor = anchor
        # Keep the input on screen even on short logical HiDPI workareas.
        composer_height = self.composer_paper.get_preferred_height_for_width(self.width)[1] if self.composer_paper.get_visible() else 0
        toolbar_height = self.toolbar.get_preferred_height_for_width(self.width)[1]
        height = max(72, min(self.preferred_height, work[3]-composer_height-toolbar_height-16))
        if height != self.height_limit:
            was_latest = not self.scrolled
            self.height_limit = height
            self.geometry()
            if was_latest:self.set_scroll(self.maximum())
            self.window.resize(1,1)
            self.paint()
        position = place(anchor, self.window.get_size(), work, prefer_sides=True, left_limit=left_limit)
        if position != self.position:
            self.position = position
            self.window.move(*position)
        self.point_tails()

    def inspect(self):
        controls = {}
        for name, widget in {'close':self.close, 'reply_open':self.open_reply, 'reply_input':self.composer.entry, 'reply_send':self.composer.button, 'latest':self.latest, 'history':self.scroll}.items():
            xy = widget.translate_coordinates(self.window,0,0)
            a = widget.get_allocation()
            controls[name] = [*xy,a.width,a.height] if xy else None
        visible = []
        for turn,y,h in self.rows:
            widget = self.widgets.get(turn.id)
            if widget:
                visible.append({'id':turn.id, 'speaker':turn.speaker, 'text':turn.text, 'position':[self.layout.child_get_property(widget,'x'), self.layout.child_get_property(widget,'y')-self.adjustment.get_value()],
                                'size':[self.width-16,h], 'opacity':widget.get_opacity(), 'material':str(widget.assets.relative_to(self.assets.resolve())),
                                'reply_visible':False, 'actions':0, 'tail':widget.tail_state})
        content = self.message.content if self.message else None
        return {'message_id':self.message_id, 'text':content.text if content else '', 'title':content.title if content else '',
                'speaker':content.speaker if content else '', 'status':self.status.get_text(), 'theme':self.assets.name,
                'visible':self.window.get_visible(), 'position':self.position, 'size':list(self.window.get_size()),
                'window':self.window.get_window().get_xid() if self.window.get_realized() else None,
                'reply_visible':self.composer_expanded, 'reply_available':self.composer.spec is not None, 'reply_text':self.composer.entry.get_text(),
                'reply_enabled':self.composer.button.get_sensitive(), 'controls':controls, 'actions':{},
                'animating':any(source is not None for source in (self.fade,self.visibility_fade.source,self.composer_fade.source)),
                'opacity':self.surface.get_opacity(), 'composer_opacity':self.composer_paper.get_opacity(), 'closing':self.closing,
                'loading':self.loading.inspect(), 'history':visible,
                'history_count':len(self.history.turns), 'history_bytes':self.history.bytes, 'rendered_cards':len(self.widgets),
                'scrollback':self.scrolled, 'scroll_value':self.adjustment.get_value(), 'scroll_max':self.maximum(),
                'composer_tail':self.composer_paper.tail_state if self.composer_paper.get_visible() else None,
                'independent_composer':True, 'composer_material':str(self.paths['composer'].relative_to(self.assets.resolve()))}

    def destroy(self):
        if self.disposed:return
        self.hide()
        self.disposed = True
        self.settings.disconnect(self.preference_handler)
        for probe in self.probes.values():probe.destroy()
        self.composer.scroll.remove(self.composer.entry)
        self.composer.entry.destroy()
        self.window.destroy()
