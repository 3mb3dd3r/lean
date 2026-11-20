package lean.client.launch;

import net.minecraft.launchwrapper.ITweaker;
import net.minecraft.launchwrapper.LaunchClassLoader;
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;
import org.spongepowered.asm.launch.MixinBootstrap;
import org.spongepowered.asm.mixin.MixinEnvironment;
import org.spongepowered.asm.mixin.Mixins;

import java.io.File;
import java.util.ArrayList;
import java.util.List;

/**
 * Passed to LaunchWrapper with {@code --tweakClass}. It hands the game its arguments, loads
 * OptiFine's class transformer when OptiFine is on the classpath, then starts Mixin.
 *
 * <p>Order matters: OptiFine swaps in whole patched classes, so its transformer has to run
 * before Mixin's, or OptiFine would throw away our hooks.
 *
 * <p>Keep this package free of other client classes: LaunchWrapper loads it with the parent
 * class loader.
 */
public final class LeanTweaker implements ITweaker {
    private static final Logger LOG = LogManager.getLogger("Lean");
    private final List<String> args = new ArrayList<String>();

    @Override
    public void acceptOptions(List<String> args, File gameDir, File assetsDir, String version) {
        this.args.addAll(args);
        if (gameDir != null) {
            this.args.add("--gameDir");
            this.args.add(gameDir.getAbsolutePath());
        }
        if (assetsDir != null) {
            this.args.add("--assetsDir");
            this.args.add(assetsDir.getAbsolutePath());
        }
        if (version != null) {
            this.args.add("--version");
            this.args.add(version);
        }
    }

    @Override
    public void injectIntoClassLoader(LaunchClassLoader classLoader) {
        if (classLoader.getResource("optifine/OptiFineClassTransformer.class") != null) {
            LOG.info("OptiFine found on the classpath, loading its class transformer");
            // Same as OptiFine's own tweaker setup: its installer/transformer package loads with the
            // parent class loader, so the transformer can find its jar and read its patches.
            classLoader.addClassLoaderExclusion("optifine.");
            classLoader.registerTransformer("optifine.OptiFineClassTransformer");
        }
        MixinBootstrap.init();
        Mixins.addConfiguration("mixins.lean.json");
        MixinEnvironment.getDefaultEnvironment().setSide(MixinEnvironment.Side.CLIENT);
    }

    @Override
    public String getLaunchTarget() {
        return "net.minecraft.client.main.Main";
    }

    @Override
    public String[] getLaunchArguments() {
        return args.toArray(new String[0]);
    }
}
