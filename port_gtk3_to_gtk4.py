#!/usr/bin/env python3
"""Port GTK3 rust code to GTK4. Second-pass mechanical replacements."""
import re
import os

SRC_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'riff-daw', 'src')

def port_file(filepath):
    with open(filepath, 'r') as f:
        content = f.read()
    original = content

    # WindowType::Toplevel removal — in GTK4, Window::new() takes no args
    content = re.sub(r'WindowType::Toplevel,?\s*', '', content)
    content = re.sub(r'gtk4::Window::new\(\s*\)', 'gtk4::Window::new()', content)
    content = re.sub(r'Window::new\(\s*\)', 'Window::new()', content)

    # Inhibit(true/false) → bool (GTK4 signal handlers return bool)
    content = re.sub(r'\bgtk4::Inhibit\((\w+)\)', r'\1', content)
    content = re.sub(r'\bglib::Inhibit\((\w+)\)', r'\1', content)
    content = re.sub(r'\bInhibit\((\w+)\)', r'\1', content)

    # MessageDialog / MessageDialogBuilder → remove/gtk4 AlertDialog stub
    # Keep as compile-time placeholders to reduce error count
    content = content.replace('MessageDialogBuilder::new()', 'MessageDialogBuilderCompat::new()')

    # connect_delete_event → connect_close_request (callback returns bool)
    content = content.replace('.connect_delete_event(', '.connect_close_request(')

    # show()/hide() already done in first pass

    # glib::idle_add_local → needs glib::MainContext wrapper; mark for manual
    content = content.replace('glib::idle_add_local_local(', '// TODO: needs MainContext::default().invoke_local')

    if content != original:
        with open(filepath, 'w') as f:
            f.write(content)
        print(f"Ported: {filepath}")
    else:
        print(f"No changes: {filepath}")


def main():
    for fname in os.listdir(SRC_DIR):
        if fname.endswith('.rs'):
            port_file(os.path.join(SRC_DIR, fname))

if __name__ == '__main__':
    main()