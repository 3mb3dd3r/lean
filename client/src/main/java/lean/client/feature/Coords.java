package lean.client.feature;

import lean.client.LeanClient;
import lean.client.hud.HudElement;
import net.minecraft.client.entity.EntityClientPlayerMP;
import net.minecraft.util.MathHelper;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;

/** Block coordinates of your feet, and optionally the direction you face. */
public final class Coords implements HudElement {
    private static final String ID = "coords";
    private static final String[] FACING = {"South", "West", "North", "East"};

    private final LeanClient client;

    public Coords(LeanClient client) {
        this.client = client;
    }

    @Override
    public String id() {
        return ID;
    }

    @Override
    public boolean visible() {
        return client.profile().enabled(ID);
    }

    @Override
    public List<String> lines() {
        EntityClientPlayerMP player = client.mc().thePlayer;
        if (player == null) {
            return Collections.emptyList();
        }
        int x = MathHelper.floor_double(player.posX);
        int y = MathHelper.floor_double(player.boundingBox.minY);
        int z = MathHelper.floor_double(player.posZ);
        List<String> lines = new ArrayList<String>(2);
        lines.add("X: " + x + "  Y: " + y + "  Z: " + z);
        if (client.profile().flag(ID, "showDirection")) {
            int facing = MathHelper.floor_double(player.rotationYaw * 4.0F / 360.0F + 0.5D) & 3;
            lines.add("Facing: " + FACING[facing]);
        }
        return lines;
    }

    @Override
    public List<String> previewLines() {
        return client.profile().flag(ID, "showDirection")
            ? Arrays.asList("X: 0  Y: 64  Z: 0", "Facing: North")
            : Collections.singletonList("X: 0  Y: 64  Z: 0");
    }
}
