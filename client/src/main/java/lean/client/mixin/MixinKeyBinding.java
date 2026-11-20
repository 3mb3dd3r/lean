package lean.client.mixin;

import lean.client.LeanClient;
import net.minecraft.client.settings.KeyBinding;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** The game calls KeyBinding.onTick for every key or mouse button press it handles. */
@Mixin(KeyBinding.class)
public abstract class MixinKeyBinding {
    @Inject(method = "onTick", at = @At("HEAD"))
    private static void lean$onTick(int keyCode, CallbackInfo ci) {
        LeanClient.get().onKeyPress(keyCode);
    }
}
