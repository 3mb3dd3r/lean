package lean.client.dev;

import lean.client.LeanClient;
import lean.client.feature.WaypointScreen;
import lean.client.hud.HudEditorScreen;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiMainMenu;
import net.minecraft.init.Items;
import net.minecraft.item.ItemStack;
import net.minecraft.util.MathHelper;
import net.minecraft.util.ScreenShotHelper;
import net.minecraft.world.WorldSettings;
import net.minecraft.world.WorldType;

/**
 * Development check, only active with {@code -Dlean.dev.autotest=true} ({@code gradlew runClient
 * -Pautotest=true}). Opens a flat creative world, sets up something for each feature to show,
 * saves screenshots of the HUD, the HUD editor and the waypoint list to {@code screenshots/},
 * then quits.
 */
public final class AutoTest {
    private static final int TIMEOUT_TICKS = 20 * 180;
    /** Splash Potion of Healing II. */
    private static final int HEALTH_SPLASH = 16421;

    private final LeanClient client;
    private int ticks;
    private int inWorld;
    private boolean worldRequested;

    public AutoTest(LeanClient client) {
        this.client = client;
        client.mc().gameSettings.pauseOnLostFocus = false;
        LeanClient.LOG.info("Autotest enabled");
    }

    public void tick() {
        Minecraft mc = client.mc();
        if (++ticks > TIMEOUT_TICKS) {
            LeanClient.LOG.error("Autotest timed out");
            mc.shutdown();
            return;
        }
        if (mc.thePlayer == null || mc.theWorld == null) {
            if (!worldRequested && mc.currentScreen instanceof GuiMainMenu) {
                worldRequested = true;
                LeanClient.LOG.info("Autotest: opening a flat creative world");
                // The renamed game jar has no nested-class info, so the enum is named by its class file.
                mc.launchIntegratedServer("lean-autotest", "Lean autotest",
                    new WorldSettings(1L, net.minecraft.world.WorldSettings$GameType.CREATIVE, false, false, WorldType.FLAT));
            }
            return;
        }
        inWorld++;
        if (inWorld == 40) {
            for (int slot = 1; slot <= 6; slot++) {
                mc.thePlayer.inventory.mainInventory[slot] = new ItemStack(Items.potionitem, 1, HEALTH_SPLASH);
            }
            if (client.waypoints().list(mc).isEmpty()) {
                int x = MathHelper.floor_double(mc.thePlayer.posX);
                int y = MathHelper.floor_double(mc.thePlayer.boundingBox.minY);
                int z = MathHelper.floor_double(mc.thePlayer.posZ);
                client.waypoints().addAt(mc, x + 3, y, z + 30);
            }
            mc.thePlayer.rotationYaw = 0f;
            mc.thePlayer.rotationPitch = 0f;
            if (!client.toggleSprint().isToggled()) {
                client.toggleSprint().onSprintKey();
            }
        } else if (inWorld == 60) {
            // The logging config must drop messages containing ${...} (Log4Shell). If the next line
            // shows up in the log with the Java version filled in, the filter is not active.
            LeanClient.LOG.info("Autotest lookup check ${sys:java.version}");
            LeanClient.LOG.info("Autotest lookup check done");
        } else if (inWorld == 90) {
            for (int i = 0; i < 9; i++) {
                client.cps().click(true);
            }
            for (int i = 0; i < 4; i++) {
                client.cps().click(false);
            }
        } else if (inWorld == 95) {
            screenshot(mc, "autotest-hud.png");
            mc.displayGuiScreen(new HudEditorScreen(client));
        } else if (inWorld == 115) {
            screenshot(mc, "autotest-hud-editor.png");
            mc.displayGuiScreen(new WaypointScreen(client));
        } else if (inWorld == 135) {
            screenshot(mc, "autotest-waypoints.png");
            mc.displayGuiScreen(null);
            LeanClient.LOG.info("Autotest finished");
            mc.shutdown();
        }
    }

    private static void screenshot(Minecraft mc, String name) {
        ScreenShotHelper.saveScreenshot(mc.mcDataDir, name, mc.displayWidth, mc.displayHeight, mc.getFramebuffer());
        LeanClient.LOG.info("Autotest screenshot saved: screenshots/{}", name);
    }
}
