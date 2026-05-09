import sys
sys.path.append('c:\\Users\\musta\\Downloads\\RLTM\\RLUPKTools')
import rl_upk_editor
import inspect

try:
    source = inspect.getsource(rl_upk_editor.parse_export_entry)
    print(source)
except Exception as e:
    print(e)
