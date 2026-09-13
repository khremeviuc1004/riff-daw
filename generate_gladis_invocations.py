#!/usr/bin/env python3
"""Parse structs in ui.rs, map GTK3 types to GTK4, verify IDs exist in the
converted .ui files, and generate gtk4_builder_from! invocations."""

import re
import sys
import os

SRC = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'riff-daw', 'src')

# GTK3-rs type -> GTK4 type fallback when the glade class has no entry
GTK3_TO_GTK4 = {
    'Dialog': 'Window',
    'MenuItem': 'Button',
    'ToolButton': 'Button',
    'RadioToolButton': 'ToggleButton',
    'ToggleToolButton': 'ToggleButton',
    'RecentChooserMenu': 'MenuButton',
    'AboutDialog': 'AboutDialog',
}

# glade widget class -> gtk4-rs type (authoritative, since it matches the
# actual widget built by the GTK4 builder and its builder.object() type)
CLASS_TO_TYPE = {
    'GtkApplicationWindow': 'ApplicationWindow',
    'GtkBox': 'Box',
    'GtkStack': 'Stack',
    'GtkToggleButton': 'ToggleButton',
    'GtkPaned': 'Paned',
    'GtkWindow': 'Window',
    'GtkProgressBar': 'ProgressBar',
    'GtkEntry': 'Entry',
    'GtkComboBoxText': 'ComboBoxText',
    'GtkButton': 'Button',
    'GtkAboutDialog': 'AboutDialog',
    'GtkMenuButton': 'MenuButton',
    'GtkAdjustment': 'Adjustment',
    'GtkViewport': 'Viewport',
    'GtkScale': 'Scale',
    'GtkDrawingArea': 'DrawingArea',
    'GtkFrame': 'Frame',
    'GtkScrolledWindow': 'ScrolledWindow',
    'GtkLabel': 'Label',
    'GtkSeparator': 'Separator',
    'GtkGrid': 'Grid',
    'GtkSpinButton': 'SpinButton',
    'GtkScrollbar': 'Scrollbar',
    'GtkStackSwitcher': 'StackSwitcher',
    'GtkTextView': 'TextView',
    'GtkTreeView': 'TreeView',
    'GtkTreeStore': 'TreeStore',
    'GtkListStore': 'ListStore',
    'GtkTreeViewColumn': 'TreeViewColumn',
    'GtkFileFilter': 'FileFilter',
    'GtkEntryCompletion': 'EntryCompletion',
    'GtkColorButton': 'ColorButton',
    'GtkCellRendererText': 'CellRendererText',
    'GtkNotebook': 'Notebook',
    'GtkFileChooserWidget': 'FileChooserWidget',
}

# struct name -> ui file (daw.ui holds Ui, MainWindow is not from builder)
STRUCT_UI = {
    'Ui': 'daw.ui',
    'TrackPanel': 'track_panel.ui',
    'TrackDetailsDialogue': 'track_details_dialogue.ui',
    'MixerBlade': 'mixer_blade.ui',
    'RiffSetBladeHead': 'riff_set_blade_head.ui',
    'RiffSetBlade': 'riff_set_blade.ui',
    'RiffSequenceBlade': 'riff_sequence_blade.ui',
    'RiffGridBlade': 'riff_grid_blade.ui',
    'RiffArrangementBlade': 'riff_arrangement_blade.ui',
    'RiffArrangementRiffSetBlade': 'riff_arrangement_riff_set_blade.ui',
    'TrackMidiRoutingDialogue': 'track_midi_routing_dialogue.ui',
    'TrackMidiRoutingPanel': 'track_midi_routing_panel.ui',
    'TrackAudioRoutingDialogue': 'track_audio_routing_dialogue.ui',
    'TrackAudioRoutingPanel': 'track_audio_routing_panel.ui',
}

ui_dir = os.path.join(SRC)
ui_ids = {}
ui_classes = {}
for fname in os.listdir(ui_dir):
    if fname.endswith('.ui'):
        with open(os.path.join(ui_dir, fname)) as f:
            content = f.read()
        ui_ids[fname] = set(re.findall(r'id="([^"]+)"', content))
        for m in re.finditer(r'<object class="([^"]+)"[^>]*id="([^"]+)"', content):
            ui_classes.setdefault(fname, {})[m.group(2)] = m.group(1)

with open(os.path.join(SRC, 'ui.rs')) as f:
    ui_rs = f.read()

struct_re = re.compile(
    r'#\[derive\(Clone\)\]\s*'
    r'pub struct (\w+) \{(.*?)\n\}',
    re.S,
)

output = []
problems = []

for m in struct_re.finditer(ui_rs):
    name = m.group(1)
    body = m.group(2)
    if name not in STRUCT_UI:
        continue
    ui_file = STRUCT_UI[name]
    ids = ui_ids.get(ui_file, set())

    fields = []
    for fm in re.finditer(r'pub (\w+):\s*([\w:]+)', body):
        fname, ftype = fm.group(1), fm.group(2)
        ftype = ftype.strip()
        if ftype.startswith('gtk4::'):
            bare = ftype.split('::')[-1]
        else:
            bare = ftype
        # authoritative type: the widget class declared in the ui file
        cls = ui_classes.get(ui_file, {}).get(fname)
        new_type = CLASS_TO_TYPE.get(cls, GTK3_TO_GTK4.get(ftype, ftype))
        fields.append((fname, new_type))

    missing = [fn for fn, _ in fields if fn not in ids]
    if missing:
        problems.append(f"{name} ({ui_file}): missing IDs {missing[:20]}")

    entry = f"gtk4_builder_from!({name} {{\n"
    for fn, ft in fields:
        entry += f"    {fn}: {ft},\n"
    entry += "});\n"
    output.append(entry)

print("=" * 40)
print("GENERATED MACRO INVOCATIONS")
print("=" * 40)
print("\n".join(output))

if problems:
    print("=" * 40)
    print("PROBLEMS (field IDs not found in ui file):")
    print("=" * 40)
    print("\n".join(problems))