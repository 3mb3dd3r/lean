package lean.client.hud;

import lean.client.Profile;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.FontRenderer;
import net.minecraft.client.gui.ScaledResolution;
import org.lwjgl.opengl.GL11;

import java.util.List;

/** Draws HUD elements at their profile positions and works out where each one sits. */
public final class HudRenderer {
    private HudRenderer() {
    }

    public static void draw(Minecraft mc, Profile profile, List<HudElement> elements, boolean preview) {
        ScaledResolution res = new ScaledResolution(mc, mc.displayWidth, mc.displayHeight);
        FontRenderer font = mc.fontRendererObj;
        int color = profile.hudColor();
        boolean shadow = profile.hudShadow();
        GL11.glColor4f(1f, 1f, 1f, 1f);
        for (HudElement element : elements) {
            if (!element.visible()) {
                continue;
            }
            List<String> lines = preview ? previewOrLive(element) : element.lines();
            if (lines.isEmpty()) {
                continue;
            }
            int[] box = bounds(res, font, profile, element.id(), lines);
            drawLines(font, lines, box[0], box[1], color, shadow);
        }
    }

    static List<String> previewOrLive(HudElement element) {
        List<String> live = element.lines();
        return live.isEmpty() ? element.previewLines() : live;
    }

    static void drawLines(FontRenderer font, List<String> lines, int x, int y, int color, boolean shadow) {
        for (String line : lines) {
            font.drawString(line, x, y, color, shadow);
            y += font.FONT_HEIGHT + 1;
        }
    }

    /** {left, top, width, height} in scaled GUI pixels, kept fully on screen. */
    static int[] bounds(ScaledResolution res, FontRenderer font, Profile profile, String id, List<String> lines) {
        int width = 0;
        for (String line : lines) {
            width = Math.max(width, font.getStringWidth(line));
        }
        int height = lines.size() * (font.FONT_HEIGHT + 1) - 1;
        int screenW = res.getScaledWidth();
        int screenH = res.getScaledHeight();
        int x = Math.round(profile.position(id, "x") / 100f * screenW);
        int y = Math.round(profile.position(id, "y") / 100f * screenH);
        x = Math.max(0, Math.min(x, screenW - width));
        y = Math.max(0, Math.min(y, screenH - height));
        return new int[] {x, y, width, height};
    }
}
