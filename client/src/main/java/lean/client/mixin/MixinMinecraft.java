package lean.client.mixin;

import lean.client.LeanClient;
import net.minecraft.client.Minecraft;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(Minecraft.class)
public abstract class MixinMinecraft {
    @Inject(method = "startGame", at = @At("RETURN"))
    private void lean$startGame(CallbackInfo ci) {
        LeanClient.get().init((Minecraft) (Object) this);
    }

    @Inject(method = "runTick", at = @At("HEAD"))
    private void lean$runTick(CallbackInfo ci) {
        LeanClient.get().onTick();
    }
}
