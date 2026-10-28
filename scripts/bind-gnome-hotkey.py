#!/usr/bin/env python3
"""Optional GNOME shortcut registration. Preserves existing custom shortcuts."""
import ast
import subprocess
from pathlib import Path
schema='org.gnome.settings-daemon.plugins.media-keys'
base='/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/'
path=base+'scribiva-alpha/'
command=str(Path.home()/'.local/bin/scribiva-insert-test')
if not Path(command).is_file():
    raise SystemExit('Install Scribiva first')
result=subprocess.run(['gsettings','get',schema,'custom-keybindings'],capture_output=True,text=True,check=True).stdout.strip()
if result.startswith('@as '):
    result=result[4:]
paths=ast.literal_eval(result)
item=schema+'.custom-keybinding:'+path
for name,value in [('name','Scribiva Alpha'),('command',command),('binding','<Primary><Alt>v')]:
    subprocess.run(['gsettings','set',item,name,value],check=True)
if path not in paths:
    paths.append(path)
    subprocess.run(['gsettings','set',schema,'custom-keybindings',repr(paths)],check=True)
print('Shortcut installed: Ctrl+Alt+V')
print('If another shortcut already owns the combination, change one binding in GNOME Settings.')
