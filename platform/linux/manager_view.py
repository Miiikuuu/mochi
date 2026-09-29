"""GTK Manager pages composed around controller snapshots."""
from types import MappingProxyType
from manager_page_shell import ShellPage
from manager_page_themes import ThemesPage
from manager_page_library import LibraryPage
from manager_page_connections import ConnectionsPage
from manager_page_settings import SettingsPage
from manager_page_detail import DetailPage


class PageControls:
    """Widget references live only as long as their owning page."""
    def __init__(self, name):
        self.name = name
        self.controls = {}

    def register(self, name, widget):
        self.controls[name] = widget
        return widget


class ManagerView(ShellPage, ThemesPage, LibraryPage, ConnectionsPage, SettingsPage, DetailPage):
    @property
    def controls(self):
        """Read-only compatibility lookup for existing Manager acceptance checks."""
        return MappingProxyType({**self.shell.controls, **(self.page.controls if self.page else {})})

    def new_page(self):
        self.page = PageControls(self.area+':'+self.screen+':'+str(self.detail))
