use super::{SharedState, Tab, View};

#[allow(clippy::unnecessary_wraps, clippy::redundant_pub_crate)]
mod utils {
    use packet_inspector::Packet as ProxyPacket;
    use valence_protocol::{
        Decode, Packet,
        packets::{
            handshaking::HandshakeC2s,
            login::{
                LoginCompressionS2c, LoginDisconnectS2c, LoginHelloC2s, LoginHelloS2c, LoginKeyC2s,
                LoginQueryRequestS2c, LoginQueryResponseC2s, LoginSuccessS2c,
            },
            play::{
                AdvancementTabC2s, AdvancementUpdateS2c, BlockBreakingProgressS2c,
                BlockEntityUpdateS2c, BlockEventS2c, BlockUpdateS2c, BoatPaddleStateC2s,
                BookUpdateC2s, BossBarS2c, BundleSplitterS2c, ButtonClickC2s, ChatMessageC2s,
                ChatMessageS2c, ChatSuggestionsS2c, ChunkBiomeDataS2c, ChunkDataS2c,
                ChunkDeltaUpdateS2c, ChunkLoadDistanceS2c, ChunkRenderDistanceCenterS2c,
                ClearTitleS2c, ClickSlotC2s, ClientCommandC2s, ClientSettingsC2s, ClientStatusC2s,
                CloseHandledScreenC2s, CloseScreenS2c, CommandExecutionC2s, CommandSuggestionsS2c,
                CommandTreeS2c, CooldownUpdateS2c, CraftFailedResponseS2c, CraftRequestC2s,
                CreativeInventoryActionC2s, CustomPayloadC2s, CustomPayloadS2c, DamageTiltS2c,
                DeathMessageS2c, DifficultyS2c, DisconnectS2c, EndCombatS2c, EnterCombatS2c,
                EntitiesDestroyS2c, EntityAnimationS2c, EntityAttachS2c, EntityAttributesS2c,
                EntityDamageS2c, EntityEquipmentUpdateS2c, EntityPassengersSetS2c,
                EntityPositionS2c, EntitySetHeadYawS2c, EntitySpawnS2c, EntityStatusEffectS2c,
                EntityStatusS2c, EntityTrackerUpdateS2c, EntityVelocityUpdateS2c,
                ExperienceBarUpdateS2c, ExperienceOrbSpawnS2c, ExplosionS2c, FeaturesS2c, FullC2s,
                GameJoinS2c, GameMessageS2c, GameStateChangeS2c, HandSwingC2s, HealthUpdateS2c,
                InventoryS2c, ItemPickupAnimationS2c, JigsawGeneratingC2s, KeepAliveC2s,
                KeepAliveS2c, LightUpdateS2c, LookAndOnGroundC2s, LookAtS2c, MapUpdateS2c,
                MessageAcknowledgmentC2s, MoveRelativeS2c, NbtQueryResponseS2c, OnGroundOnlyC2s,
                OpenHorseScreenS2c, OpenScreenS2c, OpenWrittenBookS2c, OverlayMessageS2c,
                ParticleS2c, PickFromInventoryC2s, PlayPingS2c, PlayPongC2s,
                PlaySoundFromEntityS2c, PlaySoundS2c, PlayerAbilitiesS2c, PlayerActionC2s,
                PlayerActionResponseS2c, PlayerInputC2s, PlayerInteractBlockC2s,
                PlayerInteractEntityC2s, PlayerInteractItemC2s, PlayerListHeaderS2c, PlayerListS2c,
                PlayerPositionLookS2c, PlayerRemoveS2c, PlayerRespawnS2c, PlayerSessionC2s,
                PlayerSpawnPositionS2c, PlayerSpawnS2c, PositionAndOnGroundC2s,
                ProfilelessChatMessageS2c, QueryBlockNbtC2s, QueryEntityNbtC2s, RecipeBookDataC2s,
                RecipeCategoryOptionsC2s, RemoveEntityStatusEffectS2c, RemoveMessageS2c,
                RenameItemC2s, RequestCommandCompletionsC2s, ResourcePackSendS2c,
                ResourcePackStatusC2s, RotateAndMoveRelativeS2c, RotateS2c, ScoreboardDisplayS2c,
                ScoreboardObjectiveUpdateS2c, ScoreboardPlayerUpdateS2c,
                ScreenHandlerPropertyUpdateS2c, ScreenHandlerSlotUpdateS2c,
                SelectAdvancementTabS2c, SelectMerchantTradeC2s, ServerMetadataS2c,
                SetCameraEntityS2c, SetTradeOffersS2c, SignEditorOpenS2c, SimulationDistanceS2c,
                SpectatorTeleportC2s, StatisticsS2c, StopSoundS2c, SubtitleS2c,
                SynchronizeRecipesS2c, SynchronizeTagsS2c, TeamS2c, TeleportConfirmC2s,
                TitleFadeS2c, TitleS2c, UnloadChunkS2c, UnlockRecipesS2c, UpdateBeaconC2s,
                UpdateCommandBlockC2s, UpdateCommandBlockMinecartC2s, UpdateDifficultyC2s,
                UpdateDifficultyLockC2s, UpdateJigsawC2s, UpdatePlayerAbilitiesC2s,
                UpdateSelectedSlotC2s, UpdateSelectedSlotS2c, UpdateSignC2s,
                UpdateStructureBlockC2s, VehicleMoveC2s, VehicleMoveS2c,
                WorldBorderCenterChangedS2c, WorldBorderInitializeS2c,
                WorldBorderInterpolateSizeS2c, WorldBorderSizeChangedS2c,
                WorldBorderWarningBlocksChangedS2c, WorldBorderWarningTimeChangedS2c,
                WorldEventS2c, WorldTimeUpdateS2c,
            },
            status::{QueryPingC2s, QueryPongS2c, QueryRequestC2s, QueryResponseS2c},
        },
    };

    include!(concat!(env!("OUT_DIR"), "/packet_to_string.rs"));
}

pub struct TextView {
    last_packet_id: Option<usize>,
    packet_str: String,
}

impl Tab for TextView {
    fn new() -> Self {
        Self {
            last_packet_id: None,
            packet_str: String::new(),
        }
    }

    fn name(&self) -> &'static str {
        "Text Viewer"
    }
}

impl View for TextView {
    #[allow(clippy::significant_drop_tightening)]
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut SharedState) {
        let packets = state.packets.read().unwrap();
        let Some(packet_index) = state.selected_packet else {
            self.last_packet_id = None;
            self.packet_str = String::new();
            return;
        };

        if self.last_packet_id != Some(packet_index) {
            self.last_packet_id = Some(packet_index);

            self.packet_str = utils::packet_to_string(&packets[packet_index])
                .unwrap_or_else(|err| format!("Error: {err}"));
        }

        code_view_ui(ui, &self.packet_str);
    }
}

// From: https://github.com/emilk/egui/blob/master/crates/egui_demo_lib/src/syntax_highlighting.rs

use egui::text::LayoutJob;

/// View some code with syntax highlighting and selection.
pub fn code_view_ui(ui: &mut egui::Ui, mut code: &str) {
    let language = "rs";
    let theme = CodeTheme::from_memory(ui.ctx());

    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
        let mut layout_job = highlight(ui.ctx(), &theme, buffer.as_str(), language);
        layout_job.wrap.max_width = wrap_width; // no wrapping
        ui.ctx().fonts_mut(|f| f.layout_job(layout_job))
    };

    ui.add(
        egui::TextEdit::multiline(&mut code)
            .font(egui::TextStyle::Monospace) // for cursor height
            .code_editor()
            .desired_width(ui.available_width())
            .desired_rows(24)
            .lock_focus(true)
            .layouter(&mut layouter),
    );
}

/// Memoized Code highlighting
pub fn highlight(ctx: &egui::Context, theme: &CodeTheme, code: &str, language: &str) -> LayoutJob {
    type HighlightCache = egui::cache::FrameCache<LayoutJob, Highlighter>;

    ctx.memory_mut(|mem| {
        mem.caches
            .cache::<HighlightCache>()
            .get((theme, code, language))
            .clone()
    })
}

// ----------------------------------------------------------------------------

#[derive(Clone, Copy, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[allow(unused)]
enum SyntectTheme {
    Base16EightiesDark,
    Base16MochaDark,
    Base16OceanDark,
    Base16OceanLight,
    InspiredGitHub,
    SolarizedDark,
    SolarizedLight,
}

#[allow(unused)]
impl SyntectTheme {
    fn all() -> impl ExactSizeIterator<Item = Self> {
        [
            Self::Base16EightiesDark,
            Self::Base16MochaDark,
            Self::Base16OceanDark,
            Self::Base16OceanLight,
            Self::InspiredGitHub,
            Self::SolarizedDark,
            Self::SolarizedLight,
        ]
        .iter()
        .copied()
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Base16EightiesDark => "Base16 Eighties (dark)",
            Self::Base16MochaDark => "Base16 Mocha (dark)",
            Self::Base16OceanDark => "Base16 Ocean (dark)",
            Self::Base16OceanLight => "Base16 Ocean (light)",
            Self::InspiredGitHub => "InspiredGitHub (light)",
            Self::SolarizedDark => "Solarized (dark)",
            Self::SolarizedLight => "Solarized (light)",
        }
    }

    const fn syntect_key_name(self) -> &'static str {
        match self {
            Self::Base16EightiesDark => "base16-eighties.dark",
            Self::Base16MochaDark => "base16-mocha.dark",
            Self::Base16OceanDark => "base16-ocean.dark",
            Self::Base16OceanLight => "base16-ocean.light",
            Self::InspiredGitHub => "InspiredGitHub",
            Self::SolarizedDark => "Solarized (dark)",
            Self::SolarizedLight => "Solarized (light)",
        }
    }

    pub(crate) const fn is_dark(self) -> bool {
        match self {
            Self::Base16EightiesDark
            | Self::Base16MochaDark
            | Self::Base16OceanDark
            | Self::SolarizedDark => true,

            Self::Base16OceanLight | Self::InspiredGitHub | Self::SolarizedLight => false,
        }
    }
}

#[derive(Clone, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct CodeTheme {
    dark_mode: bool,

    syntect_theme: SyntectTheme,
}

impl Default for CodeTheme {
    fn default() -> Self {
        Self::dark()
    }
}

#[allow(unused)]
impl CodeTheme {
    pub(crate) const fn from_style(style: &egui::Style) -> Self {
        if style.visuals.dark_mode {
            Self::dark()
        } else {
            Self::light()
        }
    }

    pub(crate) fn from_memory(ctx: &egui::Context) -> Self {
        if ctx.theme() == egui::Theme::Dark {
            ctx.data_mut(|d| {
                d.get_persisted(egui::Id::new("dark"))
                    .unwrap_or_else(Self::dark)
            })
        } else {
            ctx.data_mut(|d| {
                d.get_persisted(egui::Id::new("light"))
                    .unwrap_or_else(Self::light)
            })
        }
    }

    pub(crate) fn store_in_memory(self, ctx: &egui::Context) {
        if self.dark_mode {
            ctx.data_mut(|d| d.insert_persisted(egui::Id::new("dark"), self));
        } else {
            ctx.data_mut(|d| d.insert_persisted(egui::Id::new("light"), self));
        }
    }
}

#[allow(unused)]
impl CodeTheme {
    pub(crate) const fn dark() -> Self {
        Self {
            dark_mode: true,
            syntect_theme: SyntectTheme::SolarizedDark,
        }
    }

    pub(crate) const fn light() -> Self {
        Self {
            dark_mode: false,
            syntect_theme: SyntectTheme::SolarizedLight,
        }
    }

    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) {
        // Renamed in egui 0.32 when "dark/light" grew a third, follow-the-system
        // option; the deprecated shim is gone as of 0.35.
        egui::widgets::global_theme_preference_buttons(ui);

        for theme in SyntectTheme::all() {
            if theme.is_dark() == self.dark_mode {
                ui.radio_value(&mut self.syntect_theme, theme, theme.name());
            }
        }
    }
}

// ----------------------------------------------------------------------------

struct Highlighter {
    ps: syntect::parsing::SyntaxSet,
    ts: syntect::highlighting::ThemeSet,
}

impl Default for Highlighter {
    fn default() -> Self {
        Self {
            ps: syntect::parsing::SyntaxSet::load_defaults_newlines(),
            ts: syntect::highlighting::ThemeSet::load_defaults(),
        }
    }
}

impl egui::cache::ComputerMut<(&CodeTheme, &str, &str), LayoutJob> for Highlighter {
    fn compute(&mut self, (theme, code, lang): (&CodeTheme, &str, &str)) -> LayoutJob {
        self.highlight(theme, code, lang)
    }
}

impl Highlighter {
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    fn highlight(&self, theme: &CodeTheme, code: &str, lang: &str) -> LayoutJob {
        self.highlight_impl(theme, code, lang).unwrap_or_else(|| {
            // Fallback:
            LayoutJob::simple(
                code.into(),
                egui::FontId::monospace(12.0),
                if theme.dark_mode {
                    egui::Color32::LIGHT_GRAY
                } else {
                    egui::Color32::DARK_GRAY
                },
                f32::INFINITY,
            )
        })
    }

    fn highlight_impl(&self, theme: &CodeTheme, text: &str, language: &str) -> Option<LayoutJob> {
        use egui::text::{LayoutSection, TextFormat};
        use syntect::{easy::HighlightLines, highlighting::FontStyle, util::LinesWithEndings};

        let syntax = self
            .ps
            .find_syntax_by_name(language)
            .or_else(|| self.ps.find_syntax_by_extension(language))?;

        let theme = theme.syntect_theme.syntect_key_name();
        let mut h = HighlightLines::new(syntax, &self.ts.themes[theme]);

        let mut job = LayoutJob {
            text: text.into(),
            ..Default::default()
        };

        for line in LinesWithEndings::from(text) {
            for (style, range) in h.highlight_line(line, &self.ps).ok()? {
                let fg = style.foreground;
                let text_color = egui::Color32::from_rgb(fg.r, fg.g, fg.b);
                let italics = style.font_style.contains(FontStyle::ITALIC);
                let underline = style.font_style.contains(FontStyle::ITALIC);
                let underline = if underline {
                    egui::Stroke::new(1.0, text_color)
                } else {
                    egui::Stroke::NONE
                };
                job.sections.push(LayoutSection {
                    leading_space: 0.0,
                    byte_range: as_byte_range(text, range),
                    format: TextFormat {
                        font_id: egui::FontId::monospace(12.0),
                        color: text_color,
                        italics,
                        underline,
                        ..Default::default()
                    },
                });
            }
        }

        Some(job)
    }
}

fn as_byte_range(whole: &str, range: &str) -> egui::text::ByteRange {
    let whole_start = whole.as_ptr() as usize;
    let range_start = range.as_ptr() as usize;
    assert!(whole_start <= range_start);
    assert!(range_start + range.len() <= whole_start + whole.len());
    let offset = range_start - whole_start;
    egui::text::ByteIndex(offset)..egui::text::ByteIndex(offset + range.len())
}
