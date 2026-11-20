package lean.client;

import lean.client.dev.AutoTest;
import lean.client.feature.Coords;
import lean.client.feature.CpsCounter;
import lean.client.feature.PotCounter;
import lean.client.feature.ToggleSprint;
import lean.client.feature.WaypointScreen;
import lean.client.feature.Waypoints;
import lean.client.hud.HudEditorScreen;
import lean.client.hud.HudElement;
import lean.client.hud.HudRenderer;
import net.minecraft.client.Minecraft;
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

import java.io.File;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;

/**
 * Ties the features to the game. The Mixin hooks in {@code lean.client.mixin} call into this
 * class; everything runs on the game's main thread.
 */
public final class LeanClient {
    public static final Logger LOG = LogManager.getLogger("Lean");
    private static final LeanClient INSTANCE = new LeanClient();

    private Minecraft mc;
    private Profile profile;
    private ToggleSprint toggleSprint;
    private CpsCounter cps;
    private PotCounter pots;
    private Waypoints waypoints;
    private List<HudElement> hud = Collections.emptyList();
    private AutoTest autoTest;
    private boolean ready;

    private LeanClient() {
    }

    public static LeanClient get() {
        return INSTANCE;
    }

    /** Called once, after the game window and resources are ready. */
    public void init(Minecraft mc) {
        this.mc = mc;
        File dir = new File(mc.mcDataDir, "lean");
        profile = Profile.load(new File(dir, "profile.json"));
        toggleSprint = new ToggleSprint(this);
        cps = new CpsCounter(this);
        pots = new PotCounter(this);
        waypoints = new Waypoints(new File(dir, "waypoints.json"));
        hud = Arrays.asList(toggleSprint, cps, new Coords(this), pots);
        // Dev only: -Dlean.dev.autotest=true, or LEAN_DEV_AUTOTEST=1 when started by the launcher.
        if (Boolean.getBoolean("lean.dev.autotest") || "1".equals(System.getenv("LEAN_DEV_AUTOTEST"))) {
            autoTest = new AutoTest(this);
        }
        ready = true;
        LOG.info("Lean client ready, profile '{}' ({})", profile.name(), profile.id());
    }

    /** Start of every game tick (20 per second). */
    public void onTick() {
        if (!ready) {
            return;
        }
        toggleSprint.tick();
        pots.tick();
        if (autoTest != null) {
            autoTest.tick();
        }
    }

    /**
     * A key or mouse button was pressed. Mouse buttons arrive as {@code button - 100}, so left
     * click is -100 and right click is -99, the same way the game's key bindings see them.
     */
    public void onKeyPress(int keyCode) {
        if (!ready || mc.currentScreen != null || keyCode == 0) {
            return;
        }
        if (keyCode == -100) {
            cps.click(true);
        } else if (keyCode == -99) {
            cps.click(false);
        }
        if (keyCode == mc.gameSettings.keyBindSprint.getKeyCode()) {
            toggleSprint.onSprintKey();
        }
        if (keyCode == profile.key("hudEditor")) {
            mc.displayGuiScreen(new HudEditorScreen(this));
        } else if (profile.enabled("waypoints")) {
            if (keyCode == profile.key("addWaypoint")) {
                waypoints.addHere(mc);
            } else if (keyCode == profile.key("waypointMenu")) {
                mc.displayGuiScreen(new WaypointScreen(this));
            }
        }
    }

    /** End of the in-game HUD pass. */
    public void onRenderHud() {
        if (!ready || mc.currentScreen instanceof HudEditorScreen) {
            return;
        }
        HudRenderer.draw(mc, profile, hud, false);
    }

    /** World pass, right before the hand is drawn. */
    public void onRenderWorld(float partialTicks) {
        if (ready && profile.enabled("waypoints")) {
            waypoints.render(mc, partialTicks, profile.flag("waypoints", "showDistance"));
        }
    }

    public Minecraft mc() {
        return mc;
    }

    public Profile profile() {
        return profile;
    }

    public List<HudElement> hud() {
        return hud;
    }

    public Waypoints waypoints() {
        return waypoints;
    }

    public CpsCounter cps() {
        return cps;
    }

    public ToggleSprint toggleSprint() {
        return toggleSprint;
    }
}
