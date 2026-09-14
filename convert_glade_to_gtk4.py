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
    'GtkImage': {'icon-size'},
    'GtkFrame': {'shadow-type', 'label-yalign', 'border-width'},
    'GtkScrolledWindow': {'shadow-type', 'border-width', 'window-placement', 'window-placement-set', 'events'},
    'GtkButton': {'use-stock', 'relief', 'always-show-image'},
    'GtkToggleButton': {'always-show-image', 'relief'},
    'GtkToolButton': set(),
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
    'GtkMenuItem': {'mnemonic-widget', 'submenu',
                    'name', 'action-name', 'accel-path', 'use-stock'},
    'GtkImageMenuItem': {'submenu', 'always-show-image', 'use-stock'},
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

# GTK3 stock icon names -> GTK4 (freedesktop) icon names. Non-symbolic
# names are used: this app renders under full-colour icon themes (breeze,
# mate) where several *-symbolic names do not exist.
STOCK_ICON_MAP = {
    'gtk-add': 'list-add',
    'gtk-remove': 'list-remove',
    'gtk-delete': 'edit-delete',
    'gtk-copy': 'edit-copy',
    'gtk-paste': 'edit-paste',
    'gtk-cut': 'edit-cut',
    'gtk-save': 'document-save',
    'gtk-save-as': 'document-save-as',
    'gtk-open': 'document-open',
    'gtk-close': 'window-close',
    'gtk-edit': 'accessories-text-editor',
    'gtk-about': 'help-about',
    'gtk-index': 'help-about',
    'gtk-preferences': 'preferences-system',
    'gtk-media-play': 'media-playback-start',
    'gtk-media-pause': 'media-playback-pause',
    'gtk-media-stop': 'media-playback-stop',
    'gtk-media-record': 'media-record',
    'gtk-media-next': 'media-skip-forward',
    'gtk-media-previous': 'media-skip-backward',
    'gtk-zoom-in': 'zoom-in',
    'gtk-zoom-out': 'zoom-out',
    'gtk-zoom-fit': 'zoom-fit-best',
    'gtk-convert': 'media-playlist-repeat',
    'gtk-refresh': 'view-refresh',
    'gtk-justify-left': 'format-justify-left',
    'gtk-jump-to': 'go-jump',
    'gtk-goto-first': 'go-first',
    'gtk-select-all': 'edit-select-all',
    'gtk-orientation-landscape': 'video-display',
    'gtk-orientation-portrait': 'phone',
    'gtk-ok': 'emblem-ok',
    'gtk-apply': 'emblem-ok',
    'gtk-cancel': 'process-cancel-symbolic',
    'gtk-clear': 'edit-clear-all',
    'gtk-find': 'edit-find',
    'gtk-connect': 'network-connect',
    'gtk-disconnect': 'network-offline',
    'gtk-execute': 'system-run',
    'gtk-info': 'dialog-information',
    'gtk-dialog-warning': 'dialog-warning',
    'gtk-dialog-error': 'dialog-error',
    'gtk-dialog-question': 'dialog-question',
    'gtk-leave-fullscreen': 'view-restore',
}

# GTK3 stock ids used as labels -> the display text GTK3 rendered
STOCK_LABEL_MAP = {
    'gtk-new': '_New',
    'gtk-open': '_Open',
    'gtk-save': '_Save',
    'gtk-save-as': 'Save _As',
    'gtk-quit': '_Quit',
    'gtk-close': '_Close',
    'gtk-cancel': '_Cancel',
    'gtk-ok': '_OK',
    'gtk-yes': '_Yes',
    'gtk-no': '_No',
    'gtk-cut': 'Cu_t',
    'gtk-copy': '_Copy',
    'gtk-paste': '_Paste',
    'gtk-delete': '_Delete',
    'gtk-select-all': 'Select _All',
    'gtk-edit': '_Edit',
    'gtk-find': '_Find',
    'gtk-about': '_About',
    'gtk-preferences': '_Preferences',
    'gtk-index': '_Contents',
    'gtk-apply': '_Apply',
    'gtk-revert-to-saved': '_Revert',
}

# top-level <object class="GtkImage" id="..."> elements of the file being
# converted, so buttons referencing them through the GTK3 `image` property
# can resolve their icon (reset by convert_file for every input file).
NAMED_IMAGES = {}


def stock_icon(image_el):
    """The GTK4 icon name a GTK3 GtkImage element should carry."""
    stock = (image_el.findtext('property[@name="stock"]') or '').strip()
    if stock:
        return STOCK_ICON_MAP.get(stock, stock.replace('gtk-', ''))
    return (image_el.findtext('property[@name="icon-name"]') or '').strip() or None


def button_image_icon(image_ref):
    """Icon for the top-level GtkImage a GTK3 button referenced through its
    now-removed `image` property. (Buttons with an *inline* GtkImage keep
    that child, whose own stock property is converted separately.)"""
    if image_ref and image_ref in NAMED_IMAGES:
        return stock_icon(NAMED_IMAGES[image_ref])
    return None


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
        # In GTK3 a GtkButton/GtkToggleButton takes its icon from an
        # `<child internal-child="image">` GtkImage (or from a stock id);
        # GTK4 dropped both, replacing them with a single `icon-name`
        # property. Keep the icon instead of discarding it.
        if pname == 'image' and cls in ('GtkButton', 'GtkToggleButton', 'GtkRadioButton'):
            if not obj.findall('property[@name="icon-name"]'):
                icon = button_image_icon((prop.text or '').strip())
                if icon:
                    prop.set('name', 'icon-name')
                    prop.text = icon
                    continue
            remove_props.append(prop)
            continue
        # Gtk(MenuItem|ImageMenuItem) -> GtkButton: a stock label id
        # ('gtk-new' + use-stock) becomes real display text; a plain label
        # is valid GtkButton text and stays. The `image` property is gone
        # in GTK4 and menu rows are text-only by convention.
        if pname == 'label' and cls in ('GtkButton', 'GtkToggleButton', 'GtkRadioButton') \
                and (prop.text or '').strip().startswith('gtk-'):
            # raw stock id used as a button label -> real display text
            # (mnemonic underscores dropped: these glade buttons do not set
            # use-underline)
            label = STOCK_LABEL_MAP.get(prop.text.strip())
            if label:
                prop.text = label.replace('_', '')
                prop.set('translatable', 'yes')
                continue
        if cls in ('GtkMenuItem', 'GtkImageMenuItem'):
            if pname == 'image':
                remove_props.append(prop)
                continue
            if pname == 'label' and (prop.text or '').strip().startswith('gtk-'):
                stock = prop.text.strip()
                label = STOCK_LABEL_MAP.get(stock)
                if label:
                    prop.text = label
                    prop.set('translatable', 'yes')
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
                stock = (prop.text or '').strip()
                prop.set('name', 'icon-name')
                prop.text = STOCK_ICON_MAP.get(stock, stock.replace('gtk-', ''))
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

    # GTK4 GtkBuilder cannot parse <packing> at all (`Unhandled tag`), so
    # drop it unconditionally. The placement it carried (GtkStack page
    # names, GtkGrid cell attaches, GtkPaned resize/shrink hints) is
    # re-injected from the .glade source by restore_packing_gtk4.py.
    packing = child.find('packing')
    if packing is not None:
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

    # Index top-level GtkImage objects first: GTK3 buttons reference them by
    # builder id through the `image` property, which has no GTK4 equivalent.
    NAMED_IMAGES.clear()
    for child in root:
        if child.tag == 'object' and child.get('class') == 'GtkImage' and child.get('id'):
            NAMED_IMAGES[child.get('id')] = child

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