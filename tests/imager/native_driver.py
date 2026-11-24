"""Small WebDriver harness for the Linux desktop test build. Run under dbus-run-session."""
import base64
import contextlib
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import tempfile
import time
from urllib.error import HTTPError
from urllib.request import Request, urlopen


def wait(fn, description, timeout=15):
    end = time.monotonic() + timeout
    last = ''
    while time.monotonic() < end:
        try:
            result = fn()
            if result:
                return result
        except (ValueError, OSError, RuntimeError) as exc:
            last = str(exc)
        time.sleep(.12)
    raise AssertionError(f'{description}: {last}')


class Desktop:
    def __init__(self, port):
        self.url = f'http://127.0.0.1:{port}'
        self.session = ''

    def call(self, verb, path, body=None):
        data = json.dumps(body).encode() if body is not None else None
        req = Request(self.url + path, data, {'Content-Type':'application/json'}, method=verb)
        try:
            with urlopen(req, timeout=45) as response:
                return json.load(response)['value']
        except HTTPError as exc:
            raise RuntimeError(exc.read().decode()[:1200]) from exc

    def js(self, script, *args, asynchronous=False):
        return self.call('POST',self.session+'/execute/'+('async' if asynchronous else 'sync'),{'script':script,'args':args})

    def shot(self, path):
        path.write_bytes(base64.b64decode(self.call('GET', self.session+'/screenshot')))

    def find(self,selector,label=None):
        return self.js("return [...document.querySelectorAll(arguments[0])].find(e=>e.getClientRects().length&&!e.matches(':disabled')&&(!arguments[1]||e.innerText.trim()===arguments[1]||e.getAttribute('aria-label')===arguments[1]))||null",selector,label)

    def click(self,selector,label=None):
        e=wait(lambda:self.find(selector,label),str((selector,label)))
        # WebKit's default scroll may leave controls behind the fixed action bar.
        self.js("arguments[0].scrollIntoView({block:'center',inline:'nearest',behavior:'instant'})",e)
        self.call('POST',self.session+'/element/'+e['element-6066-11e4-a52e-4f735466cecf']+'/click',{})

    def shown(self, text):
        return wait(lambda:self.js('return document.body.innerText.includes(arguments[0])',text),text)

    def ipc(self,name,args):
        result=self.js("const cb=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1]).then(x=>cb({ok:true,x})).catch(e=>cb({ok:false,error:String(e)}))",name,args,asynchronous=True)
        if not result['ok']:
            raise RuntimeError(result['error'])
        return result.get('x')


@contextlib.contextmanager
def launch(binary, desktop_scale=1):
    binary=Path(binary).resolve(strict=True)
    if desktop_scale not in (1, 2):
        raise ValueError("desktop_scale must be 1 or 2")
    driver=Path.home()/'.cargo/bin/tauri-driver'
    with tempfile.TemporaryDirectory(prefix='imager-r3-') as td, contextlib.ExitStack() as stack:
        temp=Path(td)
        def clean(proc):
            proc.terminate()
            try: proc.wait(5)
            except subprocess.TimeoutExpired:
                proc.kill();proc.wait()
        screen=f'{1600 * desktop_scale}x{1000 * desktop_scale}x24'
        # Let Xvfb allocate its own display atomically when several suites start together.
        read_fd, write_fd = os.pipe()
        with os.fdopen(read_fd, 'r', encoding='ascii') as reader:
            with os.fdopen(write_fd, 'w', encoding='ascii') as writer:
                x=subprocess.Popen(['Xvfb','-displayfd',str(writer.fileno()),'-screen','0',screen,'-nolisten','tcp'],
                                   pass_fds=(writer.fileno(),),stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
                stack.callback(clean,x)
            if not select.select([reader], [], [], 15)[0]:
                raise RuntimeError('Xvfb timed out allocating a display')
            display = reader.readline().strip()
            if not display.isdecimal():
                raise RuntimeError('Xvfb did not allocate a display')
        env=dict(os.environ,DISPLAY=f':{display}',GDK_BACKEND='x11',GDK_SCALE=str(desktop_scale),GDK_DPI_SCALE='1',WEBKIT_DISABLE_DMABUF_RENDERER='1',
                 LYNX_TEST_DISABLE_HARDWARE_AUTOSTART='1',LYNX_TEST_ALLOW_PARALLEL_INSTANCE='1',
                 LYNX_TEST_AUTH_KEYRING_NAMESPACE=temp.name,LYNX_TEST_SUPERVISOR_KEYRING_NAMESPACE=temp.name)
        for key in ('HOME','XDG_CONFIG_HOME','XDG_DATA_HOME','XDG_CACHE_HOME','XDG_STATE_HOME'):
            folder=temp/key; folder.mkdir();env[key]=str(folder)
        subprocess.run(['dbus-update-activation-environment','DISPLAY','HOME','GDK_BACKEND','XDG_CONFIG_HOME','XDG_DATA_HOME','XDG_CACHE_HOME','XDG_STATE_HOME'],env=env,check=True)
        def port():
            with socket.socket() as s:
                s.bind(('127.0.0.1',0))
                return s.getsockname()[1]
        http,native=port(),port()
        d=subprocess.Popen([str(driver),'--port',str(http),'--native-port',str(native)],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        stack.callback(clean,d)
        web=Desktop(http)
        wait(lambda:d.poll() is None and web.call('GET','/status'),'tauri-driver')
        session=web.call('POST','/session',{'capabilities':{'alwaysMatch':{'tauri:options':{'application':str(binary)}}}})
        # Tauri negotiated the native WebKit session. Send the remaining W3C
        # commands directly to that session, including non-idempotent clicks.
        web=Desktop(native)
        web.session='/session/'+session['sessionId']
        stack.callback(web.call,'DELETE',web.session)
        web.call('POST',web.session+'/window/rect',{'width':1290,'height':790,'x':0,'y':0})
        wait(lambda:web.js("return location.origin==='tauri://localhost' && !!document.querySelector('.imager-app-header')"),'native basic workspace',40)
        yield web,temp
