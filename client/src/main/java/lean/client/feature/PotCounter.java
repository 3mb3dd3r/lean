package lean.client.feature;

import lean.client.LeanClient;
import lean.client.hud.HudElement;
import net.minecraft.client.entity.EntityClientPlayerMP;
import net.minecraft.init.Items;
import net.minecraft.item.ItemPotion;
import net.minecraft.item.ItemStack;
import net.minecraft.potion.Potion;
import net.minecraft.potion.PotionEffect;

import java.util.Collections;
import java.util.List;

/** Number of splash Instant Health potions in your inventory, recounted every tick. */
public final class PotCounter implements HudElement {
    private static final String ID = "potCounter";

    private final LeanClient client;
    private int count;

    public PotCounter(LeanClient client) {
        this.client = client;
    }

    public void tick() {
        EntityClientPlayerMP player = client.mc().thePlayer;
        if (player == null || !client.profile().enabled(ID)) {
            count = 0;
            return;
        }
        int total = 0;
        for (ItemStack stack : player.inventory.mainInventory) {
            if (isHealthSplash(stack)) {
                total += stack.stackSize;
            }
        }
        count = total;
    }

    static boolean isHealthSplash(ItemStack stack) {
        if (stack == null || stack.getItem() != Items.potionitem || !ItemPotion.isSplash(stack.getMetadata())) {
            return false;
        }
        List<?> effects = Items.potionitem.getEffects(stack);
        if (effects == null) {
            return false;
        }
        for (Object effect : effects) {
            if (((PotionEffect) effect).getPotionID() == Potion.heal.id) {
                return true;
            }
        }
        return false;
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
        return client.mc().thePlayer == null ? Collections.<String>emptyList() : Collections.singletonList("Pots: " + count);
    }

    @Override
    public List<String> previewLines() {
        return Collections.singletonList("Pots: 32");
    }
}
