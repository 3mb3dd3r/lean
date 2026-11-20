package lean.client.mixin;

import lean.client.LeanClient;
import net.minecraft.client.renderer.EntityRenderer;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Draws waypoints after the world and before the hand, while the world camera is still set up. */
@Mixin(EntityRenderer.class)
public abstract class MixinEntityRenderer {
    @Inject(method = "renderWorld",
        at = @At(value = "INVOKE", target = "Lnet/minecraft/client/renderer/EntityRenderer;renderHand(FI)V"))
    private void lean$beforeHand(float partialTicks, long finishTimeNano, CallbackInfo ci) {
        LeanClient.get().onRenderWorld(partialTicks);
    }
}
