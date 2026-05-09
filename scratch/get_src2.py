import sys
sys.path.append('c:\\Users\\musta\\Downloads\\RLTM\\RLUPKTools')
import rl_upk_editor
import inspect

try:
    source = inspect.getsource(rl_upk_editor.parse_decrypted_package)
    print(source)
except Exception as e:
    print(e)
