package lean.client;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import com.google.gson.JsonPrimitive;
import org.lwjgl.input.Keyboard;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.io.Reader;
import java.io.Writer;
import java.nio.charset.Charset;

/**
 * The active settings profile ({@code <gameDir>/lean/profile.json}), written by the launcher
 * before each launch. Format: docs/launch-contract.md. Missing fields fall back to defaults,
 * and fields this class doesn't know about are kept when the file is saved again.
 */
public final class Profile {
    private static final Charset UTF8 = Charset.forName("UTF-8");
    private static final Gson GSON = new GsonBuilder().setPrettyPrinting().create();

    private final File file;
    private final JsonObject root;

    private Profile(File file, JsonObject root) {
        this.file = file;
        this.root = root;
    }

    /** Reads the profile, or starts from defaults (and writes them) if the file is missing or broken. */
    public static Profile load(File file) {
        if (file.isFile()) {
            Reader reader = null;
            try {
                reader = new InputStreamReader(new FileInputStream(file), UTF8);
                JsonElement parsed = new JsonParser().parse(reader);
                if (parsed != null && parsed.isJsonObject()) {
                    return new Profile(file, parsed.getAsJsonObject());
                }
                LeanClient.LOG.warn("{} is not a JSON object, using defaults", file);
            } catch (Exception e) {
                LeanClient.LOG.warn("Could not read {}, using defaults", file, e);
            } finally {
                closeQuietly(reader);
            }
        }
        Profile profile = new Profile(file, defaults());
        profile.save();
        return profile;
    }

    static JsonObject defaults() {
        JsonObject root = new JsonObject();
        root.addProperty("schema", 1);
        root.addProperty("id", "default");
        root.addProperty("name", "Default");
        JsonObject hud = new JsonObject();
        hud.addProperty("color", "FFFFFF");
        hud.addProperty("shadow", true);
        root.add("hud", hud);
        // Default positions stack without overlapping down to a 240-pixel-high scaled screen.
        root.add("toggleSprint", section(true, 0.5f, 1.0f, "showStatus", true));
        root.getAsJsonObject("toggleSprint").addProperty("toggled", false);
        root.add("cps", section(true, 0.5f, 5.0f, "showRight", true));
        root.add("coords", section(true, 0.5f, 9.5f, "showDirection", true));
        root.add("potCounter", section(true, 0.5f, 18.0f, null, false));
        JsonObject waypoints = new JsonObject();
        waypoints.addProperty("enabled", true);
        waypoints.addProperty("showDistance", true);
        root.add("waypoints", waypoints);
        JsonObject keys = new JsonObject();
        keys.addProperty("addWaypoint", "B");
        keys.addProperty("waypointMenu", "M");
        keys.addProperty("hudEditor", "RSHIFT");
        root.add("keys", keys);
        return root;
    }

    private static JsonObject section(boolean enabled, float x, float y, String option, boolean optionValue) {
        JsonObject s = new JsonObject();
        s.addProperty("enabled", enabled);
        if (option != null) {
            s.addProperty(option, optionValue);
        }
        s.addProperty("x", x);
        s.addProperty("y", y);
        return s;
    }

    public String id() {
        return string(root, "id", "default");
    }

    public String name() {
        return string(root, "name", "Default");
    }

    public boolean enabled(String feature) {
        return flag(feature, "enabled");
    }

    /** A boolean option of a feature section, e.g. flag("cps", "showRight"). */
    public boolean flag(String feature, String option) {
        JsonElement value = get(feature, option);
        if (value != null && value.isJsonPrimitive() && value.getAsJsonPrimitive().isBoolean()) {
            return value.getAsBoolean();
        }
        JsonElement fallback = get(defaults(), feature, option);
        return fallback != null && fallback.getAsBoolean();
    }

    public void setFlag(String feature, String option, boolean value) {
        sectionFor(feature).addProperty(option, value);
    }

    /** Position of a HUD element as a percentage of the scaled screen; axis is "x" or "y". */
    public float position(String feature, String axis) {
        JsonElement value = get(feature, axis);
        if (value != null && value.isJsonPrimitive() && value.getAsJsonPrimitive().isNumber()) {
            return clampPercent(value.getAsFloat());
        }
        JsonElement fallback = get(defaults(), feature, axis);
        return fallback != null ? fallback.getAsFloat() : 0f;
    }

    public void setPosition(String feature, float x, float y) {
        JsonObject section = sectionFor(feature);
        section.addProperty("x", round2(clampPercent(x)));
        section.addProperty("y", round2(clampPercent(y)));
    }

    public int hudColor() {
        String hex = string(root.has("hud") && root.get("hud").isJsonObject() ? root.getAsJsonObject("hud") : null, "color", "FFFFFF");
        try {
            return Integer.parseInt(hex.startsWith("#") ? hex.substring(1) : hex, 16) & 0xFFFFFF;
        } catch (NumberFormatException e) {
            return 0xFFFFFF;
        }
    }

    public boolean hudShadow() {
        return flag("hud", "shadow");
    }

    /** LWJGL key code for a key setting, or 0 when unbound or unknown. */
    public int key(String name) {
        String value = string(root.has("keys") && root.get("keys").isJsonObject() ? root.getAsJsonObject("keys") : null, name, null);
        if (value == null) {
            value = defaults().getAsJsonObject("keys").get(name).getAsString();
        }
        if (value.equalsIgnoreCase("NONE")) {
            return 0;
        }
        return Keyboard.getKeyIndex(value.toUpperCase());
    }

    public void save() {
        Writer writer = null;
        try {
            File dir = file.getParentFile();
            if (dir != null && !dir.isDirectory() && !dir.mkdirs()) {
                throw new IllegalStateException("could not create " + dir);
            }
            File tmp = new File(file.getPath() + ".tmp");
            writer = new OutputStreamWriter(new FileOutputStream(tmp), UTF8);
            GSON.toJson(root, writer);
            writer.close();
            writer = null;
            if (file.exists() && !file.delete()) {
                throw new IllegalStateException("could not replace " + file);
            }
            if (!tmp.renameTo(file)) {
                throw new IllegalStateException("could not rename " + tmp);
            }
        } catch (Exception e) {
            LeanClient.LOG.warn("Could not save {}", file, e);
        } finally {
            closeQuietly(writer);
        }
    }

    private JsonObject sectionFor(String feature) {
        JsonElement existing = root.get(feature);
        if (existing != null && existing.isJsonObject()) {
            return existing.getAsJsonObject();
        }
        JsonObject created = new JsonObject();
        root.add(feature, created);
        return created;
    }

    private JsonElement get(String feature, String option) {
        return get(root, feature, option);
    }

    private static JsonElement get(JsonObject from, String feature, String option) {
        JsonElement section = from.get(feature);
        if (section == null || !section.isJsonObject()) {
            return null;
        }
        return section.getAsJsonObject().get(option);
    }

    private static String string(JsonObject from, String key, String fallback) {
        if (from == null) {
            return fallback;
        }
        JsonElement value = from.get(key);
        if (value instanceof JsonPrimitive && ((JsonPrimitive) value).isString()) {
            return value.getAsString();
        }
        return fallback;
    }

    private static float clampPercent(float value) {
        return Math.max(0f, Math.min(100f, value));
    }

    private static float round2(float value) {
        return Math.round(value * 100f) / 100f;
    }

    private static void closeQuietly(java.io.Closeable closeable) {
        if (closeable != null) {
            try {
                closeable.close();
            } catch (Exception ignored) {
                // nothing useful to do
            }
        }
    }
}
