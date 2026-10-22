// Expõe no D-Bus da sessão o que o Wayland não deixa um app fazer: ativar a
// janela de outro. O `terraforme cli` chama `Activate` com os PIDs dos seus
// processos pais (do mais próximo ao mais distante) quando a janela flutuante
// pede uma instância; a primeira janela de um desses processos vem para
// frente, trocando de área de trabalho e saindo do minimizado.
import Gio from 'gi://Gio';
import Meta from 'gi://Meta';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const PATH = '/org/gnome/Shell/Extensions/Terraforme';
const IFACE = `<node>
  <interface name="org.gnome.Shell.Extensions.Terraforme">
    <method name="Activate">
      <arg type="au" direction="in" name="pids"/>
      <arg type="b" direction="out" name="found"/>
    </method>
  </interface>
</node>`;

export default class TerraformeExtension extends Extension {
    enable() {
        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._dbus.export(Gio.DBus.session, PATH);
    }

    disable() {
        this._dbus?.unexport();
        this._dbus = null;
    }

    Activate(pids) {
        const windows = global.get_window_actors()
            .map(actor => actor.get_meta_window())
            .filter(w => w && w.get_window_type() === Meta.WindowType.NORMAL);
        for (const pid of pids) {
            // Várias janelas do mesmo processo (ex.: VS Code): a usada por último.
            const window = windows
                .filter(w => w.get_pid() === pid)
                .sort((a, b) => b.get_user_time() - a.get_user_time())[0];
            if (window) {
                if (window.minimized)
                    window.unminimize();
                Main.activateWindow(window, global.get_current_time());
                return true;
            }
        }
        return false;
    }
}
