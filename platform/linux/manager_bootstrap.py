"""Build configuration and resource snapshots before attaching GTK views."""
from concurrent.futures import ThreadPoolExecutor
from manager_controller import ManagerController


class ControllerBootstrap:
    def __init__(self, dispatch):
        self.dispatch = dispatch
        self.worker = ThreadPoolExecutor(max_workers=1, thread_name_prefix='mochi-startup')

    def start(self, completed):
        def load():
            try:
                controller = ManagerController()
                controller.adopt_resources(controller.load_resources())
                self.dispatch(completed, controller, None)
            except Exception as error:
                self.dispatch(completed, None, error)
        self.worker.submit(load)

    def close(self):
        self.worker.shutdown(wait=False)
