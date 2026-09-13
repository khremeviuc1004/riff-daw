#!/usr/bin/env python3
"""Convert GTK3 .glade XML files to GTK4 .ui XML files."""

import xml.etree.ElementTree as ET
import sys
import os

CLASS_MAP = {
    'GtkToolButton': 'GtkButton',
    'GtkRadioToolButton': 'GtkToggleButton',
    'GtkToggleToolButton': 'GtkToggleButton',
    'GtkSeparatorToolItem': 'GtkSeparator',
    'GtkButtonBox': 'GtkBox',
    'GtkMenu': 'GtkBox',
    'GtkMenuItem': 'GtkButton',
    'GtkImageMenuItem': 'GtkButton',
    'GtkSeparatorMenuItem': 'GtkSeparator',
    'GtkDialog': 'GtkWindow',
    'GtkMessageDialog': 'GtkWindow',
    'GtkRecentFilter': 'GtkFileFilter',
    'GtkRecentChooserMenu': 'GtkMenuButton',
    'GtkRecentFilter': 'GtkFileFilter',
    'GtkMenuButton': 'GtkButton',
    'GtkRadioButton': 'GtkToggleButton',
    'GtkMenuBar': 'GtkBox',
    'GtkToolbar': 'GtkBox',
}

REMOVE_PROPERTIES = {
    'GtkImage': {'stock', 'icon-size', 'pixel-size'},
    'GtkFrame': {'shadow-type', 'label-yalign', 'border-width'},
    'GtkScrolledWindow': {'shadow-type', 'border-width', 'window-placement', 'window-placement-set', 'events'},
    'GtkButton': {'use-stock', 'image', 'relief', 'always-show-image'},
    'GtkToggleButton': {'always-show-image', 'relief', 'image'},
    'GtkToolButton': {'image'},
    'GtkWindow': {'type-hint', 'skip-taskbar-hint', 'destroy-with-parent',
                  'border-width', 'position', 'window-position'},
    'GtkDialog': {'type-hint', 'skip-taskbar-hint', 'use-header-bar',
                  'destroy-with-parent', 'border-width'},
    'GtkAboutDialog': {'type-hint', 'skip-taskbar-hint', 'destroy-with-parent'},
    'GtkBox': {'border-width'},
    'GtkGrid': {'border-width'},
    'GtkEntry': {'shadow-type', 'border-width', 'populate-all'},
    'GtkLabel': {'xpad', 'ypad', 'xalign', 'yalign'},
    'GtkDrawingArea': {'events'},
    'GtkButtonBox': {'layout-style', 'spacing'},
    'GtkToolbar': {'toolbar-style', 'show-arrow', 'icon-size', 'icon_size'},
    'GtkToolItem': {'expand', 'homogeneous', 'visible-horizontal', 'visible-vertical'},
    'GtkSeparatorToolItem': {'draw', 'expand'},
    'GtkMenuItem': {'label', 'use-underline', 'mnemonic-widget', 'submenu',
                    'name', 'action-name', 'accel-path', 'use-stock', 'image'},
    'GtkImageMenuItem': {'label', 'use-underline', 'submenu', 'always-show-image', 'use-stock', 'image'},
    'GtkCheckMenuItem': {'draw-as-radio'},
    'GtkSeparatorMenuItem': {'set-expanded'},
    'GtkComboBoxText': {'has-entry'},
    'GtkRecentChooserMenu': {'filter', 'limit', 'sort-type'},
    'GtkPopover': {'modal', 'position'},
    'GtkStackSwitcher': {'homogeneous', 'icon-size'},
    'Notebook': {},
}

GLOBAL_REMOVE_PROPERTIES = {
    'can-focus',
    'receives-default',
    'has-tooltip',
    'app-paintable',
    'composite-child',
    'double-buffered',
    'expand',
    'fill',
    'padding',
    'pack-type',
    'position',
    'resize-mode',
    'no-show-all',
    'margin-left',
    'margin-right',
    'window-position',
}

# Properties that need renaming (old, new) - empty string to just remove
PROP_RENAME = {
    'can-default': 'receives-default',
    'label-selectable': 'selectable',
    'wrap-mode': 'wrap-mode',
}


def fix_object(obj, hoist_target=None):
    """Recursively transform a <object> element in place."""
    cls = obj.get('class', '')
    new_cls = CLASS_MAP.get(cls, cls)

    # GtkTreeSelection is implicit in GTK4 (via selection model); drop it.
    if cls == 'GtkTreeSelection':
        return None

    # Handle removed containers: GtkToolItem and GtkSeparatorToolItem wrap a child
    # object under a <child> or <property name="child"> element. Inline that child.
    if cls in ('GtkToolItem', 'GtkSeparatorToolItem'):
        # find the wrapped child object
        child_obj = None
        for prop in obj.findall('property'):
            if prop.get('name') == 'child':
                child_obj = prop.find('object')
                if child_obj is not None:
                    obj.remove(prop)
        if child_obj is None:
            for child in obj.findall('child'):
                child_obj = child.find('object')
                if child_obj is not None:
                    obj.remove(child)
        if child_obj is not None:
            fix_object(child_obj, hoist_target)
            return child_obj
        return None

    obj.set('class', new_cls)

    # Backward: if a child class name needs renaming to a removed parent, we
    # might need to re-process. Handle properties first.
    remove_props = []
    for prop in obj.findall('property'):
        pname = prop.get('name', '')
        if pname in REMOVE_PROPERTIES.get(cls, set()) or pname in GLOBAL_REMOVE_PROPERTIES:
            remove_props.append(prop)
            continue
        if pname in PROP_RENAME:
            new_name = PROP_RENAME[pname]
            if new_name:
                prop.set('name', new_name)
            else:
                remove_props.append(prop)

    for prop in remove_props:
        obj.remove(prop)

    # For GtkImage stock items, convert stock property into icon-name
    if cls == 'GtkImage':
        for prop in obj.findall('property'):
            if prop.get('name') == 'stock':
                prop.set('name', 'icon-name')

    # GTK3 stock icon names -> GTK4 icon names
    STOCK_ICON_MAP = {
        'gtk-save': 'document-save',
        'gtk-dialog-warning': 'dialog-warning',
        'gtk-edit': 'document-edit-symbolic',
        'gtk-goto-first': 'go-first-symbolic',
        'gtk-find': 'edit-find-symbolic',
        'gtk-select-all': 'edit-select-all',
        'gtk-leave-fullscreen': 'view-restore',
        'gtk-jump-to': 'go-jump',
        'gtk-disconnect': 'network-offline',
        'gtk-zoom-out': 'zoom-out-symbolic',
        'gtk-zoom-in': 'zoom-in-symbolic',
        'gtk-media-record': 'media-record-symbolic',
        'gtk-add': 'list-add-symbolic',
        'gtk-ok': 'ok-symbolic',
        'gtk-close': 'window-close-symbolic',
        'gtk-cancel': 'dialog-cancel-symbolic',
        'gtk-delete': 'edit-delete-symbolic',
    }
    if cls in ('GtkToolButton', 'GtkToggleToolButton', 'GtkRadioToolButton',
               'GtkMenuItem', 'GtkImageMenuItem', 'GtkButton',
               'GtkToggleButton', 'GtkCheckMenuItem'):
        for prop in obj.findall('property'):
            if prop.get('name') in ('stock-id', 'stock'):
                icon_name = STOCK_ICON_MAP.get(prop.text, None)
                if icon_name:
                    prop.set('name', 'icon-name')
                    prop.text = icon_name
                else:
                    remove_props.append(prop)

    # Process children (recursively)
    for child in list(obj):
        if child.tag == 'object':
            fixed = fix_object(child, hoist_target)
            if fixed is None:
                obj.remove(child)
            elif fixed is not child:
                idx = list(obj).index(child)
                obj.remove(child)
                obj.insert(idx, fixed)
        elif child.tag == 'placeholder':
            obj.remove(child)
        elif child.tag == 'action-widgets':
            obj.remove(child)
        elif child.tag == 'signal':
            pass
        elif child.tag == 'child':
            res = fix_child(child, obj.get('class'), hoist_target)
            if res == 'remove':
                obj.remove(child)

    return obj


def fix_child(child, parent_cls=None, hoist_target=None):
    """Process a <child> element (packing props, nested objects)."""
    # GTK4 has no internal children on windows/dialogs. Convert the internal
    # vbox/action_area wrappers into plain children (keeps the widget tree).
    ic = child.get('internal-child')
    if ic:
        del child.attrib['internal-child']
    # GTK3 gtkmenu item "submenu" child type does not exist in GTK4
    if child.get('type') == 'submenu':
        del child.attrib['type']

    # GTK4 GtkComboBoxText no longer has a public internal entry child. The
    # entry objects referenced from Rust (fields `*_entry`) are hoisted to the
    # top-level interface root so their IDs remain available to the builder.
    if parent_cls == 'GtkComboBoxText' and ic == 'entry':
        if hoist_target is not None:
            entry_obj = child.find('object')
            if entry_obj is not None:
                for p in list(entry_obj):
                    if p.tag != 'property':
                        entry_obj.remove(p)
                    else:
                        # apply the same property removal rules as fix_object
                        pname = p.get('name', '')
                        if pname in REMOVE_PROPERTIES.get('GtkEntry', set()) \
                                or pname in GLOBAL_REMOVE_PROPERTIES:
                            entry_obj.remove(p)
                entry_obj.set('class', 'GtkEntry')
                hoist_target.append(entry_obj)
        return 'remove'

    # Remove packing properties in GTK4
    packing = child.find('packing')
    if packing is not None:
        for prop in list(packing):
            if prop.get('name', '') in GLOBAL_REMOVE_PROPERTIES:
                packing.remove(prop)
        if len(packing) == 0 or all(not list(p) for p in packing):
            child.remove(packing)
    for obj in list(child):
        if obj.tag == 'object':
            fixed = fix_object(obj, hoist_target)
            if fixed is None:
                child.remove(obj)
            elif fixed is not obj:
                idx = list(child).index(obj)
                child.remove(obj)
                child.insert(idx, fixed)
    return False


def has_children_with_content(elem):
    return len(list(elem)) > 0


def convert_file(input_path, output_path):
    ET.register_namespace('', 'http://www.gtk.org/introspection/1.0')

    tree = ET.parse(input_path)
    root = tree.getroot()

    # Fix requires tag
    for req in root.findall('requires'):
        req.set('version', '4.0')
        if req.get('lib') == 'gtk+':
            req.set('lib', 'gtk')
    # Add requires if missing
    if len(root.findall('requires')) == 0:
        requires = ET.SubElement(root, 'requires')
        requires.set('lib', 'gtk')
        requires.set('version', '4.0')

    for child in list(root):
        if child.tag == 'object':
            fixed = fix_object(child, root)
            if fixed is None:
                root.remove(child)

    ET.indent(tree, space='  ')
    tree.write(output_path, xml_declaration=True, encoding='UTF-8')
    print(f"Converted: {input_path} -> {output_path}")


def main():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    glade_dir = os.path.join(script_dir, 'riff-daw', 'src')

    glade_files = [f for f in os.listdir(glade_dir) if f.endswith('.glade')]
    glade_files.sort()

    for fname in glade_files:
        # Only process files we haven't already generated a good .ui for
        input_path = os.path.join(glade_dir, fname)
        output_path = os.path.join(glade_dir, fname.replace('.glade', '.ui'))
        try:
            convert_file(input_path, output_path)
        except Exception as e:
            print(f"ERROR converting {fname}: {e}")


if __name__ == '__main__':
    main()