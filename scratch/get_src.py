import sys
sys.path.append('c:\\Users\\musta\\Downloads\\RLTM\\RLUPKTools')
import rl_upk_editor
import inspect

# Get the source code for the ParsedPackage class and print it
try:
    source = inspect.getsource(rl_upk_editor.ParsedPackage)
    print(source)
except Exception as e:
    print(e)
