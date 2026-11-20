package lean.client.feature;

import lean.client.LeanClient;
import lean.client.hud.HudElement;
import net.minecraft.client.Minecraft;
import net.minecraft.client.settings.KeyBinding;
import org.lwjgl.input.Keyboard;
import org.lwjgl.input.Mouse;

import java.util.Collections;
import java.util.List;

/**
 * Pressing the game's sprint key turns sprinting on until it is pressed again. While on, the
 * sprint key is held down for the game every tick, so all of vanilla's sprint rules still apply.
 */
public final class ToggleSprint implements HudElement {
    private static final String ID = "toggleSprint";
    private static final List<String> STATUS = Collections.singletonList("[Sprinting (Toggled)]");

    private final LeanClient client;

    public ToggleSprint(LeanClient client) {
        this.client = client;
    }

    public void onSprintKey() {
        if (!client.profile().enabled(ID)) {
            return;
        }
        boolean toggled = !isToggled();
        client.profile().setFlag(ID, "toggled", toggled);
        client.profile().save();
        if (!toggled) {
            int code = client.mc().gameSettings.keyBindSprint.getKeyCode();
            KeyBinding.setKeyBindState(code, physicallyDown(code));
        }
    }

    public void tick() {
        Minecraft mc = client.mc();
        if (mc.thePlayer == null || mc.currentScreen != null || !client.profile().enabled(ID) || !isToggled()) {
            return;
        }
        KeyBinding.setKeyBindState(mc.gameSettings.keyBindSprint.getKeyCode(), true);
    }

    public boolean isToggled() {
        return client.profile().flag(ID, "toggled");
    }

    private static boolean physicallyDown(int code) {
        return code < 0 ? Mouse.isButtonDown(code + 100) : code > 0 && Keyboard.isKeyDown(code);
    }

    @Override
    public String id() {
        return ID;
    }

    @Override
    public boolean visible() {
        return client.profile().enabled(ID) && client.profile().flag(ID, "showStatus");
    }

    @Override
    public List<String> lines() {
        return isToggled() ? STATUS : Collections.<String>emptyList();
    }

    @Override
    public List<String> previewLines() {
        return STATUS;
    }
}
