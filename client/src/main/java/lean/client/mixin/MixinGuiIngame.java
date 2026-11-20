package lean.client.mixin;

import lean.client.LeanClient;
import net.minecraft.client.gui.GuiIngame;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(GuiIngame.class)
public abstract class MixinGuiIngame {
    @Inject(method = "renderGameOverlay", at = @At("RETURN"))
    private void lean$renderGameOverlay(float partialTicks, boolean hasScreen, int mouseX, int mouseY, CallbackInfo ci) {
        LeanClient.get().onRenderHud();
    }
}
