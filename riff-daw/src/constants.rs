pub const GTK_APPLICATION_ID: &str = "au.com.pwcu.daw";

pub const VST24: &str = "VST24";
pub const VST24_CHECKER_EXECUTABLE_NAME: &str = "vst_checker";
pub const VST_PATH_ENVIRONMENT_VARIABLE_NAME: &str = "VST_PATH";
pub const CLAP: &str = "CLAP";
pub const CLAP_CHECKER_EXECUTABLE_NAME: &str = "clap_checker";
pub const CLAP_PATH_ENVIRONMENT_VARIABLE_NAME: &str = "CLAP_PATH";
pub const VST3: &str = "VST3";
pub const VST3a: &str = "VST3a";
pub const VST3_CHECKER_EXECUTABLE_NAME: &str = "vst3_checker";
pub const VST3_PATH_ENVIRONMENT_VARIABLE_NAME: &str = "VST3_PATH";

pub const TRACK_VIEW_TRACK_PANEL_HEIGHT: i32 = 19;
pub const RIFF_SET_VIEW_TRACK_PANEL_HEIGHT: i32 = 51;
pub const RIFF_SEQUENCE_VIEW_TRACK_PANEL_HEIGHT: i32 = 51;
pub const RIFF_ARRANGEMENT_VIEW_TRACK_PANEL_HEIGHT: i32 = 51;

pub const NOTE_NAMES: [&str; 12] = ["C","C#/Db","D","D#/Eb","E","F","F#/Gb","G","G#/Ab","A","A#/Bb","B"];

pub const CONFIGURATION_FILE_NAME: &str = "riff-daw.conf";


pub const LUA_GLOBAL_STATE: &str = "state";

pub const DAW_AUTO_SAVE_THREAD_NAME: &str = "DAW autosave";

pub const EVENT_DELETION_BEAT_TOLERANCE: f64 = 0.05;

pub const BLOCK_SIZE_MAX: i32 = 2048;
pub const EVENT_BUFFER_SIZE: usize = 1024;

pub const PLUGIN_PATHS_SEPARATOR: &str = ",";

pub const MUSICAL_ITEM_LENGTH_OPTIONS: [&str; 26] = [
    "10",
    "9",
    "8",
    "7",
    "6",
    "5",
    "4",
    "3",
    "2",
    "1.5",
    "1",
    "1/2.",
    "1/2",
    "1/4.",
    "1/4 triplet",
    "1/4",
    "1/8.",
    "1/8 triplet",
    "1/8",
    "1/16.",
    "1/16 triplet",
    "1/16",
    "1/32.",
    "1/32",
    "1/64.",
    "1/64",
];

pub const NOTE_SUBDIVISIONS: [&str; 2] = ["Normal", "Triplet"];
pub const TRIPLETS: [&str; 3] = ["1/4 triplet", "1/8 triplet", "1/16 triplet"];

pub const CONTROLLER_TYPES: [(i32, &str); 69] = [
    (0, "Bank select (coarse)"),
    (1, "Modulation wheel (coarse)"),
    (2, "Breath controller (coarse)"),
    (4, "Foot controller (coarse)"),
    (5, "Portamento time (coarse)"),
    (6, "Data entry (coarse)"),
    (7, "Channel volume (coarse)"),
    (8, "Balance (coarse)"),
    (10, "Pan (coarse)"),
    (11, "Expression (coarse)2"),
    (12, "Effect control 1 (coarse)"),
    (13, "Effect control 2 (coarse)"),
    (16, "General purpose controller 1 (coarse)"),
    (17, "General purpose controller 2 (coarse)"),
    (18, "General purpose controller 3 (coarse)"),
    (19, "General purpose controller 4 (coarse)"),
    (32, "Bank select (fine)"),
    (33, "Modulation wheel (fine)"),
    (34, "Breath controller (fine)"),
    (36, "Foot controller (fine)"),
    (37, "Portamento time (fine)"),
    (38, "Data entry (fine)"),
    (39, "Channel volume (fine) (formerly main volume)"),
    (40, "Balance (fine)"),
    (42, "Pan (fine)"),
    (43, "Expression (fine)2"),
    (44, "Effect control 1 (fine)"),
    (45, "Effect control 2 (fine)"),
    (64, "Hold (damper, sustain) pedal 1 (on or off) = 64 is on"),
    (65, "Portamento pedal (on or off) = 64 is on"),
    (66, "Sostenuto pedal (on or off) = 64 is on"),
    (67, "Soft pedal (on or off) = 64 is on"),
    (68, "legato pedal (on or off) = 64 is on"),
    (69, "Hold pedal 2 (on//off) = 64 is on"),
    (70, "sound variation"),
    (71, "timbre or harmonic intensity or filter resonance"),
    (72, "release time"),
    (73, "attack time"),
    (74, "brightness or cutoff frequency"),
    (75, "decay time"),
    (76, "vibrato rate"),
    (77, "vibrato depth"),
    (78, "vibrato delay"),
    (79, "undefined"),
    (80, "General purpose controller 5"),
    (81, "General purpose controller 6"),
    (82, "General purpose controller 7"),
    (83, "General purpose controller 8"),
    (84, "Portamento control"),
    (88, "High resolution velocity prefix"),
    (91, "Effect 1 depth (default is reverb send level"),
    (92, "Effect 2 depth (formerly tremolo depth)"),
    (93, "Effect 3 depth (default is chorus send level"),
    (94, "Effect 4 depth (formerly celeste depth)"),
    (95, "Effect 5 depth (formerly phaser level)"),
    (96, "Data button increment"),
    (97, "Data button decrement"),
    (98, "Non-registered parameter (coarse)"),
    (99, "Non-registered parameter (fine)"),
    (100, "Registered parameter (coarse)"),
    (101, "Registered parameter (fine)"),
    (120, "All sound off 0"),
    (121, "All controllers off 0"),
    (122, "Local control (on or off) 0 off, 127 on"),
    (123, "All notes off 0"),
    (124, "Omni mode off 0"),
    (125, "Omni mode on 0"),
    (126, "Mono operation and all notes off"),
    (127, "Poly operation and all notes off 0"),
];



