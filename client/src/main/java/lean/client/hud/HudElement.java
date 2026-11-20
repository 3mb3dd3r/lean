package lean.client.hud;

import java.util.List;

/** A block of text on the HUD whose position and on/off state live in the profile. */
public interface HudElement {
    /** Profile section name, e.g. "cps". */
    String id();

    /** Whether the profile has this element turned on. */
    boolean visible();

    /** Lines to draw right now; empty when there is nothing to show. */
    List<String> lines();

    /** Lines shown in the HUD editor, so the element can be placed even when it is empty. */
    List<String> previewLines();
}
