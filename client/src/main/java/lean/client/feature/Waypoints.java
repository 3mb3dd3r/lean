package lean.client.feature;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import lean.client.LeanClient;
import net.minecraft.client.Minecraft;
import net.minecraft.client.entity.EntityClientPlayerMP;
import net.minecraft.client.gui.FontRenderer;
import net.minecraft.client.multiplayer.ServerData;
import net.minecraft.client.renderer.Tessellator;
import net.minecraft.client.renderer.entity.RenderManager;
import net.minecraft.util.ChatComponentText;
import net.minecraft.util.MathHelper;
import org.lwjgl.opengl.GL11;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.io.Reader;
import java.io.Writer;
import java.nio.charset.Charset;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Waypoints, stored per server (or per singleplayer world) in {@code lean/waypoints.json}.
 * Each one is drawn as a label that stays readable at any distance and shows through walls.
 */
public final class Waypoints {
    private static final Charset UTF8 = Charset.forName("UTF-8");
    private static final Gson GSON = new GsonBuilder().setPrettyPrinting().create();
    private static final int[] COLORS = {0x55FFFF, 0xFF5555, 0x55FF55, 0xFFFF55, 0xFF55FF, 0xFFAA00};
    /** Labels further away than this are drawn at this distance, in the right direction. */
    private static final double LABEL_DISTANCE = 8.0;

    private final File file;
    private final Map<String, List<Waypoint>> byWorld = new LinkedHashMap<String, List<Waypoint>>();

    public static final class Waypoint {
        public final String name;
        public final int x;
        public final int y;
        public final int z;
        public final int dimension;
        public final int color;

        Waypoint(String name, int x, int y, int z, int dimension, int color) {
            this.name = name;
            this.x = x;
            this.y = y;
            this.z = z;
            this.dimension = dimension;
            this.color = color;
        }
    }

    public Waypoints(File file) {
        this.file = file;
        load();
    }

    /** Where the player is: "mp:<server address>" or "sp:<world folder>". */
    public static String worldKey(Minecraft mc) {
        if (mc.isSingleplayer() && mc.getIntegratedServer() != null) {
            return "sp:" + mc.getIntegratedServer().getFolderName();
        }
        ServerData server = mc.getCurrentServerData();
        return server != null && server.serverIP != null ? "mp:" + server.serverIP.toLowerCase() : "unknown";
    }

    public List<Waypoint> list(Minecraft mc) {
        List<Waypoint> list = byWorld.get(worldKey(mc));
        return list == null ? Collections.<Waypoint>emptyList() : Collections.unmodifiableList(list);
    }

    public void addHere(Minecraft mc) {
        EntityClientPlayerMP player = mc.thePlayer;
        if (player == null) {
            return;
        }
        addAt(mc, MathHelper.floor_double(player.posX), MathHelper.floor_double(player.boundingBox.minY), MathHelper.floor_double(player.posZ));
    }

    public Waypoint addAt(Minecraft mc, int x, int y, int z) {
        String key = worldKey(mc);
        List<Waypoint> list = byWorld.get(key);
        if (list == null) {
            list = new ArrayList<Waypoint>();
            byWorld.put(key, list);
        }
        Waypoint waypoint = new Waypoint("Waypoint " + (list.size() + 1), x, y, z, mc.thePlayer.dimension, COLORS[list.size() % COLORS.length]);
        list.add(waypoint);
        save();
        mc.thePlayer.addChatMessage(new ChatComponentText("§7[§6Lean§7] Added §f" + waypoint.name
            + "§7 at " + x + ", " + y + ", " + z));
        return waypoint;
    }

    public void remove(Minecraft mc, Waypoint waypoint) {
        List<Waypoint> list = byWorld.get(worldKey(mc));
        if (list != null && list.remove(waypoint)) {
            save();
        }
    }

    public void render(Minecraft mc, float partialTicks, boolean showDistance) {
        EntityClientPlayerMP player = mc.thePlayer;
        if (player == null) {
            return;
        }
        List<Waypoint> list = byWorld.get(worldKey(mc));
        if (list == null || list.isEmpty()) {
            return;
        }
        GL11.glPushAttrib(GL11.GL_ALL_ATTRIB_BITS);
        mc.entityRenderer.disableLightmap(partialTicks);
        GL11.glDisable(GL11.GL_LIGHTING);
        GL11.glDisable(GL11.GL_FOG);
        GL11.glDisable(GL11.GL_DEPTH_TEST);
        GL11.glDepthMask(false);
        GL11.glEnable(GL11.GL_BLEND);
        GL11.glBlendFunc(GL11.GL_SRC_ALPHA, GL11.GL_ONE_MINUS_SRC_ALPHA);
        for (Waypoint waypoint : list) {
            if (waypoint.dimension == player.dimension) {
                drawLabel(mc, waypoint, showDistance);
            }
        }
        GL11.glPopAttrib();
    }

    private static void drawLabel(Minecraft mc, Waypoint waypoint, boolean showDistance) {
        RenderManager rm = RenderManager.instance;
        double dx = waypoint.x + 0.5 - RenderManager.renderPosX;
        double dy = waypoint.y + 1.5 - RenderManager.renderPosY;
        double dz = waypoint.z + 0.5 - RenderManager.renderPosZ;
        double distance = Math.sqrt(dx * dx + dy * dy + dz * dz);
        double drawAt = Math.min(distance, LABEL_DISTANCE);
        if (distance > 0.001) {
            dx *= drawAt / distance;
            dy *= drawAt / distance;
            dz *= drawAt / distance;
        }
        // Same on-screen size at every distance: scale grows with how far away it is drawn.
        float scale = (float) (0.0065 * Math.max(drawAt, 1.0));
        String label = showDistance ? waypoint.name + " [" + Math.round(distance) + "m]" : waypoint.name;
        FontRenderer font = mc.fontRendererObj;
        int half = font.getStringWidth(label) / 2;

        GL11.glPushMatrix();
        GL11.glTranslated(dx, dy, dz);
        GL11.glNormal3f(0f, 1f, 0f);
        GL11.glRotatef(-rm.playerViewY, 0f, 1f, 0f);
        GL11.glRotatef((mc.gameSettings.thirdPersonView == 2 ? -1 : 1) * rm.playerViewX, 1f, 0f, 0f);
        GL11.glScalef(-scale, -scale, scale);

        GL11.glDisable(GL11.GL_TEXTURE_2D);
        Tessellator tessellator = Tessellator.instance;
        tessellator.startDrawingQuads();
        tessellator.setColorRGBA_F(0f, 0f, 0f, 0.35f);
        tessellator.addVertex(-half - 2, -2, 0);
        tessellator.addVertex(-half - 2, 9, 0);
        tessellator.addVertex(half + 2, 9, 0);
        tessellator.addVertex(half + 2, -2, 0);
        tessellator.draw();
        GL11.glEnable(GL11.GL_TEXTURE_2D);
        font.drawString(label, -half, 0, 0xFF000000 | waypoint.color);
        GL11.glPopMatrix();
    }

    private void load() {
        byWorld.clear();
        if (!file.isFile()) {
            return;
        }
        Reader reader = null;
        try {
            reader = new InputStreamReader(new FileInputStream(file), UTF8);
            JsonElement parsed = new JsonParser().parse(reader);
            JsonObject worlds = parsed.getAsJsonObject().getAsJsonObject("worlds");
            if (worlds == null) {
                return;
            }
            for (Map.Entry<String, JsonElement> entry : worlds.entrySet()) {
                List<Waypoint> list = new ArrayList<Waypoint>();
                for (JsonElement element : entry.getValue().getAsJsonArray()) {
                    JsonObject o = element.getAsJsonObject();
                    list.add(new Waypoint(o.get("name").getAsString(), o.get("x").getAsInt(), o.get("y").getAsInt(),
                        o.get("z").getAsInt(), o.has("dim") ? o.get("dim").getAsInt() : 0,
                        o.has("color") ? Integer.parseInt(o.get("color").getAsString(), 16) : COLORS[0]));
                }
                byWorld.put(entry.getKey(), list);
            }
        } catch (Exception e) {
            LeanClient.LOG.warn("Could not read {}", file, e);
        } finally {
            if (reader != null) {
                try {
                    reader.close();
                } catch (Exception ignored) {
                    // nothing useful to do
                }
            }
        }
    }

    private void save() {
        JsonObject worlds = new JsonObject();
        for (Map.Entry<String, List<Waypoint>> entry : byWorld.entrySet()) {
            JsonArray array = new JsonArray();
            for (Waypoint w : entry.getValue()) {
                JsonObject o = new JsonObject();
                o.addProperty("name", w.name);
                o.addProperty("x", w.x);
                o.addProperty("y", w.y);
                o.addProperty("z", w.z);
                o.addProperty("dim", w.dimension);
                o.addProperty("color", String.format("%06X", w.color));
                array.add(o);
            }
            worlds.add(entry.getKey(), array);
        }
        JsonObject root = new JsonObject();
        root.addProperty("schema", 1);
        root.add("worlds", worlds);
        Writer writer = null;
        try {
            file.getParentFile().mkdirs();
            writer = new OutputStreamWriter(new FileOutputStream(file), UTF8);
            GSON.toJson(root, writer);
        } catch (Exception e) {
            LeanClient.LOG.warn("Could not save {}", file, e);
        } finally {
            if (writer != null) {
                try {
                    writer.close();
                } catch (Exception ignored) {
                    // nothing useful to do
                }
            }
        }
    }
}
