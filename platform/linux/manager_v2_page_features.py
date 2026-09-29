"""Theme capabilities and Translation settings."""
from manager_v2_common import Page, text
from manager_i18n import _


from manager_v2_feature_card import FeatureCard
from manager_v2_translation_settings import TranslationSettings


class FeaturesPage(Page):
    def __init__(self, *args):
        super().__init__(*args)
        self.feature_rows = {}
        self.translation_controls = {}
        self.translation = None
        self._features(self.state.draft.value)

    def update_translation(self, options):
        self.translation_options = options
        if self.translation:
            self.translation.update_translation(options)

    def update_models(self):
        if self.translation:
            self.translation.update_catalog()

    def dispose(self):
        if self.translation:
            self.translation.dispose()
        super().dispose()

    def _features(self, theme):
        self.workspace_content.append(text('Features', 'section-title'))
        self.workspace_content.append(text('Choose what this Theme can do.', 'section-hint'))
        for enabled in (True, False):
            models = [model for model in self.feature_models if model.enabled == enabled]
            if not models:
                continue
            self.workspace_content.append(text('Enabled features' if enabled else 'Available features', 'field-label'))
            for model in models:
                card = FeatureCard(model, self.layout, self.callbacks)
                self.feature_rows[model.key] = card.toggle
                self.workspace_content.append(card.root)
                if model.key == 'translation' and model.enabled:
                    self.translation = TranslationSettings(self.translation_options, self.layout, self.callbacks, self.state.models)
                    self.translation_controls = self.translation.translation_controls
                    self.workspace_content.append(self.translation.root)
        self.workspace_content.append(text('Feature changes wait for Apply.', 'field-help'))
