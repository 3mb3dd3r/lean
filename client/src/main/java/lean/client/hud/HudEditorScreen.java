package lean.client.hud;

import lean.client.LeanClient;
import lean.client.Profile;
import net.minecraft.client.gui.GuiButton;
import net.minecraft.client.gui.GuiScreen;
import net.minecraft.client.gui.ScaledResolution;

import java.util.List;

/**
 * Drag HUD elements to move them. Closing the screen (Done or Esc) saves the new positions
 * into the profile file, so the launcher shows them next time.
 */
public final class HudEditorScreen extends GuiScreen {
    private final LeanClient client;
    private HudElement dragging;
    private int grabX;
    private int grabY;

    public HudEditorScreen(LeanClient client) {
        this.client = client;
    }

    @Override
    @SuppressWarnings("unchecked")
    public void initGui() {
        buttonList.clear();
        buttonList.add(new GuiButton(0, width / 2 - 50, height - 28, 100, 20, "Done"));
    }

    @Override
    public void drawScreen(int mouseX, int mouseY, float partialTicks) {
        drawRect(0, 0, width, height, 0x55000000);
        ScaledResolution res = new ScaledResolution(mc, mc.displayWidth, mc.displayHeight);
        Profile profile = client.profile();
        for (HudElement element : client.hud()) {
            if (!element.visible()) {
                continue;
            }
            List<String> lines = HudRenderer.previewOrLive(element);
            int[] box = HudRenderer.bounds(res, fontRendererObj, profile, element.id(), lines);
            boolean hot = element == dragging || inside(box, mouseX, mouseY);
            drawRect(box[0] - 2, box[1] - 2, box[0] + box[2] + 2, box[1] + box[3] + 2, hot ? 0x60FFFFFF : 0x30FFFFFF);
            HudRenderer.drawLines(fontRendererObj, lines, box[0], box[1], profile.hudColor(), profile.hudShadow());
        }
        drawCenteredString(fontRendererObj, "Drag to move. Done or Esc saves.", width / 2, height - 42, 0xFFFFFF);
        super.drawScreen(mouseX, mouseY, partialTicks);
    }

    @Override
    protected void mouseClicked(int mouseX, int mouseY, int button) {
        if (button == 0) {
            ScaledResolution res = new ScaledResolution(mc, mc.displayWidth, mc.displayHeight);
            for (HudElement element : client.hud()) {
                if (!element.visible()) {
                    continue;
                }
                int[] box = HudRenderer.bounds(res, fontRendererObj, client.profile(), element.id(), HudRenderer.previewOrLive(element));
                if (inside(box, mouseX, mouseY)) {
                    dragging = element;
                    grabX = mouseX - box[0];
                    grabY = mouseY - box[1];
                    return;
                }
            }
        }
        super.mouseClicked(mouseX, mouseY, button);
    }

    @Override
    protected void mouseClickMove(int mouseX, int mouseY, int button, long timeSinceClick) {
        if (dragging != null) {
            float x = (mouseX - grabX) * 100f / width;
            float y = (mouseY - grabY) * 100f / height;
            client.profile().setPosition(dragging.id(), x, y);
        }
    }

    @Override
    protected void mouseReleased(int mouseX, int mouseY, int button) {
        dragging = null;
        super.mouseReleased(mouseX, mouseY, button);
    }

    @Override
    protected void actionPerformed(GuiButton button) {
        if (button.id == 0) {
            mc.displayGuiScreen(null);
        }
    }

    @Override
    public void onGuiClosed() {
        client.profile().save();
    }

    @Override
    public boolean doesGuiPauseGame() {
        return false;
    }

    private static boolean inside(int[] box, int x, int y) {
        return x >= box[0] - 2 && x <= box[0] + box[2] + 2 && y >= box[1] - 2 && y <= box[1] + box[3] + 2;
    }
}
