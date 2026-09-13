#!/usr/bin/env python3
"""Cross-check struct field types from the generated gtk4_builder_from!
invocations against the widget classes declared in the .ui files."""

import re, os, sys, subprocess, json

SRC = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'riff-daw', 'src')

# GTK4 class name -> gtk4-rs type name
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
    'GtkCellRendererToggle': 'CellRendererToggle',
    'GtkFontButton': 'FontButton',
    'GtkCheckButton': 'CheckButton',
    'GtkSwitch': 'Switch',
    'GtkSpinner': 'Spinner',
    'GtkSearchBar': 'SearchBar',
    'GtkSearchEntry': 'SearchEntry',
    'GtkToolbar': 'Toolbar',  # shouldn't exist
    'GtkIconView': 'IconView',
    'GtkFlowBox': 'FlowBox',
    'GtkPopover': 'Popover',
    'GtkMenu': 'Menu',  # shouldn't exist
}

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

# Parse the .ui files for id -> class
id_class = {}
for fname in STRUCT_UI.values():
    path = os.path.join(SRC, fname)
    with open(path) as f:
        content = f.read()
    # match <object class="..." id="...">
    for m in re.finditer(r'<object class="([^"]+)"[^>]*id="([^"]+)"', content):
        id_class[m.group(2)] = m.group(1)

# Parse invocations from generated file
with open('/tmp/opencode/invocations.txt') as f:
    content = f.read()

sections = re.findall(r'gtk4_builder_from!\((\w+) \{(.*?)\}\);', content, re.S)
problems = []
total = 0
for sname, body in sections:
    if sname not in STRUCT_UI:
        continue
    fields = re.findall(r'(\w+): ([\w]+),', body)
    for fname, ftype in fields:
        total += 1
        cls = id_class.get(fname)
        if cls is None:
            problems.append(f"{sname}.{fname}: id not found in {STRUCT_UI[sname]}")
            continue
        expected_type = CLASS_TO_TYPE.get(cls)
        if expected_type is None:
            problems.append(f"{sname}.{fname}: unknown/unconverted class {cls}")
        elif expected_type != ftype:
            problems.append(f"{sname}.{fname}: {cls} (expected {expected_type}) but struct says {ftype}")

print(f"checked {total} fields")
if problems:
    print(f"{len(problems)} problems:")
    for p in problems:
        print("  " + p)
else:
    print("ALL FIELD TYPES MATCH their .ui widget classes")