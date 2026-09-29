"""Compatibility facade for the GTK4 shell's pre-split inspection API."""
from manager_v2_shell import Shell


class ManagerV2View(Shell):
    def __getattr__(self, name):
        # Forward inspection to the owner without retaining detached page controls.
        values = self.__dict__
        for owner in ('page', 'banner', 'undo', 'commandbar'):
            target = values.get(owner)
            if target is not None and hasattr(target, name):
                return getattr(target, name)
        raise AttributeError(name)
