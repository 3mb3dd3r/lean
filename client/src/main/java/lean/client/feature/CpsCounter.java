package lean.client.feature;

import lean.client.LeanClient;
import lean.client.hud.HudElement;

import java.util.ArrayDeque;
import java.util.Collections;
import java.util.List;

/** Clicks per second over the last second, left and optionally right. Counts in-game clicks only. */
public final class CpsCounter implements HudElement {
    private static final String ID = "cps";

    private final LeanClient client;
    private final ArrayDeque<Long> left = new ArrayDeque<Long>();
    private final ArrayDeque<Long> right = new ArrayDeque<Long>();

    public CpsCounter(LeanClient client) {
        this.client = client;
    }

    public void click(boolean leftButton) {
        (leftButton ? left : right).addLast(System.currentTimeMillis());
    }

    private static int recent(ArrayDeque<Long> clicks, long now) {
        while (!clicks.isEmpty() && now - clicks.peekFirst() >= 1000L) {
            clicks.pollFirst();
        }
        return clicks.size();
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
        long now = System.currentTimeMillis();
        int l = recent(left, now);
        int r = recent(right, now);
        String text = client.profile().flag(ID, "showRight") ? l + " | " + r + " CPS" : l + " CPS";
        return Collections.singletonList(text);
    }

    @Override
    public List<String> previewLines() {
        return lines();
    }
}
