package lean.client.feature;

import lean.client.LeanClient;
import net.minecraft.client.entity.EntityClientPlayerMP;
import net.minecraft.client.gui.GuiButton;
import net.minecraft.client.gui.GuiScreen;

import java.util.List;

/** Lists this server's waypoints with distance, and lets you add or delete them. */
public final class WaypointScreen extends GuiScreen {
    private static final int ROWS = 6;
    private static final int DONE = 0;
    private static final int ADD = 1;
    private static final int PREV = 2;
    private static final int NEXT = 3;
    private static final int DELETE = 100;

    private final LeanClient client;
    private int page;

    public WaypointScreen(LeanClient client) {
        this.client = client;
    }

    @Override
    @SuppressWarnings("unchecked")
    public void initGui() {
        buttonList.clear();
        List<Waypoints.Waypoint> all = client.waypoints().list(mc);
        int pages = Math.max(1, (all.size() + ROWS - 1) / ROWS);
        page = Math.min(page, pages - 1);
        int top = 40;
        for (int row = 0; row < ROWS && page * ROWS + row < all.size(); row++) {
            buttonList.add(new GuiButton(DELETE + row, width / 2 + 100, top + row * 24, 50, 20, "Delete"));
        }
        int bottom = height - 30;
        buttonList.add(new GuiButton(ADD, width / 2 - 155, bottom, 100, 20, "Add here"));
        GuiButton prev = new GuiButton(PREV, width / 2 - 50, bottom, 45, 20, "<");
        GuiButton next = new GuiButton(NEXT, width / 2 + 5, bottom, 45, 20, ">");
        prev.enabled = page > 0;
        next.enabled = page < pages - 1;
        buttonList.add(prev);
        buttonList.add(next);
        buttonList.add(new GuiButton(DONE, width / 2 + 55, bottom, 100, 20, "Done"));
    }

    @Override
    public void drawScreen(int mouseX, int mouseY, float partialTicks) {
        drawDefaultBackground();
        drawCenteredString(fontRendererObj, "Waypoints", width / 2, 15, 0xFFFFFF);
        List<Waypoints.Waypoint> all = client.waypoints().list(mc);
        if (all.isEmpty()) {
            drawCenteredString(fontRendererObj, "No waypoints here yet.", width / 2, 60, 0xAAAAAA);
        }
        EntityClientPlayerMP player = mc.thePlayer;
        int top = 40;
        for (int row = 0; row < ROWS && page * ROWS + row < all.size(); row++) {
            Waypoints.Waypoint w = all.get(page * ROWS + row);
            int y = top + row * 24 + 6;
            String where = w.x + ", " + w.y + ", " + w.z;
            if (player != null && w.dimension == player.dimension) {
                double dx = w.x + 0.5 - player.posX;
                double dz = w.z + 0.5 - player.posZ;
                where += "  (" + Math.round(Math.sqrt(dx * dx + dz * dz)) + "m)";
            }
            fontRendererObj.drawString(w.name, width / 2 - 150, y, w.color, true);
            fontRendererObj.drawString(where, width / 2 - 40, y, 0xCCCCCC, true);
        }
        super.drawScreen(mouseX, mouseY, partialTicks);
    }

    @Override
    protected void actionPerformed(GuiButton button) {
        if (button.id == DONE) {
            mc.displayGuiScreen(null);
        } else if (button.id == ADD) {
            client.waypoints().addHere(mc);
            initGui();
        } else if (button.id == PREV) {
            page--;
            initGui();
        } else if (button.id == NEXT) {
            page++;
            initGui();
        } else if (button.id >= DELETE) {
            List<Waypoints.Waypoint> all = client.waypoints().list(mc);
            int index = page * ROWS + (button.id - DELETE);
            if (index < all.size()) {
                client.waypoints().remove(mc, all.get(index));
            }
            initGui();
        }
    }

    @Override
    public boolean doesGuiPauseGame() {
        return false;
    }
}
