import groovy.json.JsonSlurper
import java.net.URI
import java.security.MessageDigest
import java.util.UUID
import java.util.zip.ZipFile

// The client runs inside vanilla Minecraft 1.7.10 (no Forge). It is compiled against a
// readable (MCP-named) copy of the game, then renamed back to the game's obfuscated names
// so it can run in the real game. Mixin hooks it into the game; OptiFine can run next to it.

plugins {
    java
}

group = "lean"
version = "0.1.0"

val mcVersion = "1.7.10"
val mixinVersion = "0.7.11-SNAPSHOT"

java {
    toolchain {
        languageVersion.set(JavaLanguageVersion.of(8))
        // Azul Zulu: the current Temurin 8 JDK zip on GitHub is broken (Nov 2026).
        vendor.set(JvmVendorSpec.AZUL)
    }
}
val java8 = javaToolchains.launcherFor { languageVersion.set(JavaLanguageVersion.of(8)); vendor.set(JvmVendorSpec.AZUL) }

repositories {
    mavenCentral()
    maven("https://libraries.minecraft.net") { metadataSources { mavenPom(); artifact() } }
    maven("https://repo.spongepowered.org/repository/maven-public/")
    maven("https://maven.minecraftforge.net/") { metadataSources { mavenPom(); artifact() } }
}

// ---- Paths ------------------------------------------------------------------------------
val mcDir = layout.buildDirectory.dir("minecraft")
val versionJson = mcDir.map { it.file("$mcVersion.json") }
val vanillaJar = mcDir.map { it.file("minecraft-$mcVersion.jar") }
val deobfJar = mcDir.map { it.file("minecraft-$mcVersion-mcp.jar") }
val notchToMcp = mcDir.map { it.file("notch-mcp.srg") }
val mcpToNotch = mcDir.map { it.file("mcp-notch.srg") }
val mixinOut = layout.buildDirectory.dir("mixin")
val refmapName = "mixins.lean.refmap.json"
val reobfJarFile = layout.buildDirectory.file("tmp/lean-client-reobf.jar")

// ---- Dependencies -----------------------------------------------------------------------
val shade: Configuration by configurations.creating { isTransitive = false }
val mcpSrg: Configuration by configurations.creating { isTransitive = false }
val mcpNames: Configuration by configurations.creating { isTransitive = false }
val specialSource: Configuration by configurations.creating { isTransitive = false }
val launchLibs: Configuration by configurations.creating { isTransitive = false }

dependencies {
    // Libraries Minecraft 1.7.10 ships with (Mojang's 1.7.10.json). Guava is 17.0 instead of
    // 15.0 because Mixin needs it; the launcher puts 17.0 on the classpath too.
    compileOnly("org.lwjgl.lwjgl:lwjgl:2.9.1")
    compileOnly("org.lwjgl.lwjgl:lwjgl_util:2.9.1")
    compileOnly("com.google.guava:guava:17.0")
    compileOnly("com.google.code.gson:gson:2.2.4")
    compileOnly("org.apache.logging.log4j:log4j-api:2.0-beta9")
    compileOnly("org.apache.commons:commons-lang3:3.1")
    compileOnly("commons-io:commons-io:2.4")
    compileOnly("io.netty:netty-all:4.0.10.Final")
    compileOnly("com.mojang:authlib:1.5.21")
    compileOnly("net.minecraft:launchwrapper:1.12")
    compileOnly(files(deobfJar).builtBy("deobfMinecraft"))

    compileOnly("org.spongepowered:mixin:$mixinVersion") { isTransitive = false }
    shade("org.spongepowered:mixin:$mixinVersion")
    annotationProcessor("org.spongepowered:mixin:$mixinVersion")
    annotationProcessor("com.google.guava:guava:21.0")
    annotationProcessor("com.google.code.gson:gson:2.2.4")
    annotationProcessor("org.ow2.asm:asm-debug-all:5.2")
    annotationProcessor("org.apache.logging.log4j:log4j-api:2.0-beta9")
    annotationProcessor("org.apache.logging.log4j:log4j-core:2.0-beta9")
    annotationProcessor("commons-io:commons-io:2.4")
    annotationProcessor("net.minecraft:launchwrapper:1.12")

    mcpSrg("de.oceanlabs.mcp:mcp:$mcVersion:srg@zip")
    mcpNames("de.oceanlabs.mcp:mcp_stable:12-$mcVersion@zip")
    specialSource("net.md-5:SpecialSource:1.11.4:shaded")

    // Extra jars the game needs at run time (see docs/launch-contract.md).
    launchLibs("net.minecraft:launchwrapper:1.12")
    launchLibs("org.ow2.asm:asm-all:5.0.3")
    launchLibs("com.google.guava:guava:17.0")
}

// ---- Helpers ----------------------------------------------------------------------------
fun sha1Hex(f: File): String =
    MessageDigest.getInstance("SHA-1").digest(f.readBytes()).joinToString("") { "%02x".format(it) }

fun download(url: String, dest: File, sha1: String? = null) {
    if (dest.isFile && (sha1 == null || sha1Hex(dest) == sha1)) return
    dest.parentFile.mkdirs()
    val tmp = File(dest.path + ".part")
    URI(url).toURL().openStream().use { input -> tmp.outputStream().use { input.copyTo(it) } }
    if (sha1 != null && sha1Hex(tmp) != sha1) {
        tmp.delete()
        throw GradleException("SHA-1 mismatch for $url")
    }
    dest.delete()
    if (!tmp.renameTo(dest)) throw GradleException("Could not move $tmp to $dest")
}

@Suppress("UNCHECKED_CAST")
fun readJson(f: File): Map<String, Any?> = JsonSlurper().parse(f) as Map<String, Any?>

@Suppress("UNCHECKED_CAST")
fun rulesAllowWindows(rules: Any?): Boolean {
    if (rules == null) return true
    var allowed = false
    for (rule in rules as List<Map<String, Any?>>) {
        val os = rule["os"] as Map<String, Any?>?
        if (os == null || os["name"] == "windows") allowed = rule["action"] == "allow"
    }
    return allowed
}

// ---- Minecraft and mappings -------------------------------------------------------------
val downloadMinecraft by tasks.registering {
    group = "minecraft"
    description = "Downloads the vanilla $mcVersion version JSON and client jar from Mojang."
    outputs.files(versionJson, vanillaJar)
    doLast {
        val manifest = mcDir.get().file("version_manifest_v2.json").asFile
        download("https://piston-meta.mojang.com/mc/game/version_manifest_v2.json", manifest)
        @Suppress("UNCHECKED_CAST")
        val entry = (readJson(manifest)["versions"] as List<Map<String, Any?>>).first { it["id"] == mcVersion }
        download(entry["url"] as String, versionJson.get().asFile, entry["sha1"] as String)
        @Suppress("UNCHECKED_CAST")
        val client = (readJson(versionJson.get().asFile)["downloads"] as Map<String, Map<String, Any?>>)["client"]!!
        download(client["url"] as String, vanillaJar.get().asFile, client["sha1"] as String)
    }
}

val genMappings by tasks.registering {
    group = "minecraft"
    description = "Builds obfuscated<->MCP name mappings from the MCP srg and stable_12 names."
    inputs.files(mcpSrg, mcpNames)
    outputs.files(notchToMcp, mcpToNotch)
    doLast {
        val names = HashMap<String, String>()
        ZipFile(mcpNames.singleFile).use { zip ->
            for (csv in listOf("fields.csv", "methods.csv")) {
                zip.getInputStream(zip.getEntry(csv)).bufferedReader().useLines { lines ->
                    lines.drop(1).forEach { line ->
                        val cols = line.split(",")
                        if (cols.size >= 2) names[cols[0]] = cols[1]
                    }
                }
            }
        }
        val srg = ZipFile(mcpSrg.singleFile).use { zip ->
            zip.getInputStream(zip.getEntry("joined.srg")).bufferedReader().readLines()
        }
        fun named(member: String): String {
            val i = member.lastIndexOf('/')
            val name = member.substring(i + 1)
            return member.substring(0, i + 1) + (names[name] ?: name)
        }
        val toMcp = StringBuilder()
        val toNotch = StringBuilder()
        for (line in srg) {
            val p = line.trim().split(" ")
            when (p[0]) {
                "CL:" -> {
                    toMcp.append("CL: ${p[1]} ${p[2]}\n"); toNotch.append("CL: ${p[2]} ${p[1]}\n")
                }
                "FD:" -> {
                    val mcp = named(p[2])
                    toMcp.append("FD: ${p[1]} $mcp\n"); toNotch.append("FD: $mcp ${p[1]}\n")
                }
                "MD:" -> {
                    val mcp = named(p[3])
                    toMcp.append("MD: ${p[1]} ${p[2]} $mcp ${p[4]}\n"); toNotch.append("MD: $mcp ${p[4]} ${p[1]} ${p[2]}\n")
                }
            }
        }
        notchToMcp.get().asFile.writeText(toMcp.toString())
        mcpToNotch.get().asFile.writeText(toNotch.toString())
    }
}

val deobfMinecraft by tasks.registering(JavaExec::class) {
    group = "minecraft"
    description = "Renames the vanilla client jar to MCP names so the client can compile against it."
    dependsOn(downloadMinecraft, genMappings)
    inputs.files(vanillaJar, notchToMcp)
    outputs.file(deobfJar)
    classpath = specialSource
    mainClass.set("net.md_5.specialsource.SpecialSource")
    argumentProviders.add(CommandLineArgumentProvider {
        listOf(
            "--in-jar", vanillaJar.get().asFile.path,
            "--out-jar", deobfJar.get().asFile.path,
            "--srg-in", notchToMcp.get().asFile.path,
            "--quiet",
        )
    })
}

// ---- Compile, rename back, package ------------------------------------------------------
tasks.compileJava {
    dependsOn(deobfMinecraft)
    options.encoding = "UTF-8"
    val out = mixinOut.get().asFile
    outputs.dir(out)
    doFirst { out.mkdirs() }
    options.compilerArgs.addAll(
        listOf(
            "-AreobfNotchSrgFile=${mcpToNotch.get().asFile}",
            "-AoutNotchSrgFile=${File(out, "mixin-notch.srg")}",
            "-AoutRefMapFile=${File(out, refmapName)}",
            "-AdefaultObfuscationEnv=notch",
        )
    )
}

tasks.jar {
    archiveClassifier.set("dev")
    from(mixinOut.map { it.file(refmapName) })
}

val reobfJar by tasks.registering(JavaExec::class) {
    description = "Renames the compiled client from MCP names back to the game's obfuscated names."
    dependsOn(tasks.jar)
    val devJar = tasks.jar.flatMap { it.archiveFile }
    val combinedSrg = mixinOut.map { it.file("reobf.srg") }
    inputs.file(devJar)
    outputs.file(reobfJarFile)
    classpath = specialSource + configurations.compileClasspath.get()
    mainClass.set("net.md_5.specialsource.SpecialSource")
    doFirst {
        val mixinSrg = File(mixinOut.get().asFile, "mixin-notch.srg")
        combinedSrg.get().asFile.writeText(
            mcpToNotch.get().asFile.readText() + (if (mixinSrg.isFile) mixinSrg.readText() else "")
        )
        reobfJarFile.get().asFile.parentFile.mkdirs()
    }
    argumentProviders.add(CommandLineArgumentProvider {
        listOf(
            "--in-jar", devJar.get().asFile.path,
            "--out-jar", reobfJarFile.get().asFile.path,
            "--srg-in", combinedSrg.get().asFile.path,
            "--live",
            "--quiet",
        )
    })
}

val clientJar by tasks.registering(Jar::class) {
    group = "build"
    description = "Builds lean-client.jar, the jar the launcher runs (client + Mixin)."
    dependsOn(reobfJar)
    archiveBaseName.set("lean-client")
    archiveClassifier.set("")
    from(zipTree(reobfJarFile))
    from(provider { shade.map { zipTree(it) } }) {
        // Keep Mixin's runtime service files; drop only its compile-time annotation processor.
        exclude(
            "META-INF/*.SF", "META-INF/*.DSA", "META-INF/*.RSA", "META-INF/MANIFEST.MF",
            "META-INF/services/javax.annotation.processing.Processor",
            "META-INF/services/org.spongepowered.tools.obfuscation.service.IObfuscationService",
            "org/spongepowered/tools/**",
        )
    }
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    manifest { attributes("TweakClass" to "lean.client.launch.LeanTweaker", "Implementation-Version" to version) }
}

tasks.assemble { dependsOn(clientJar) }

// ---- Dev run: ./gradlew runClient [-Poptifine=<jar>] [-Pautotest=true] -------------------
val runDir = layout.projectDirectory.dir("run")
val runFiles = layout.buildDirectory.dir("run")
val libCache = File(gradle.gradleUserHomeDir, "caches/lean-mc/libraries")

val prepareRun by tasks.registering {
    group = "minecraft"
    description = "Downloads the libraries and natives the game needs and picks an assets folder."
    dependsOn(downloadMinecraft)
    outputs.upToDateWhen { false }
    doLast {
        val out = runFiles.get().asFile.apply { mkdirs() }
        val natives = File(out, "natives").apply { deleteRecursively(); mkdirs() }
        val classpath = ArrayList<String>()
        @Suppress("UNCHECKED_CAST")
        for (lib in readJson(versionJson.get().asFile)["libraries"] as List<Map<String, Any?>>) {
            if (!rulesAllowWindows(lib["rules"])) continue
            val name = lib["name"] as String
            @Suppress("UNCHECKED_CAST")
            val downloads = lib["downloads"] as Map<String, Any?>? ?: continue
            @Suppress("UNCHECKED_CAST")
            val nativesMap = lib["natives"] as Map<String, String>?
            if (nativesMap != null) {
                val classifier = nativesMap["windows"]?.replace("\${arch}", "64") ?: continue
                @Suppress("UNCHECKED_CAST")
                val art = (downloads["classifiers"] as Map<String, Map<String, Any?>>)[classifier] ?: continue
                val jar = File(libCache, art["path"] as String)
                download(art["url"] as String, jar, art["sha1"] as String)
                @Suppress("UNCHECKED_CAST")
                val exclude = ((lib["extract"] as Map<String, Any?>?)?.get("exclude") as List<String>?) ?: emptyList()
                ZipFile(jar).use { zip ->
                    zip.entries().asSequence()
                        .filter { e -> !e.isDirectory && exclude.none { e.name.startsWith(it) } }
                        .forEach { e ->
                            val f = File(natives, e.name).apply { parentFile.mkdirs() }
                            zip.getInputStream(e).use { i -> f.outputStream().use { i.copyTo(it) } }
                        }
                }
                continue
            }
            if (name.startsWith("com.google.guava:guava:")) continue // replaced by 17.0 below
            @Suppress("UNCHECKED_CAST")
            val art = downloads["artifact"] as Map<String, Any?>? ?: continue
            val jar = File(libCache, art["path"] as String)
            download(art["url"] as String, jar, art["sha1"] as String)
            classpath += jar.path
        }
        classpath += launchLibs.files.map { it.path }
        File(out, "classpath.txt").writeText(classpath.joinToString("\n"))

        val appData = System.getenv("APPDATA") ?: ""
        val assets = listOf(File(appData, ".lean/assets"), File(appData, ".minecraft/assets"))
            .firstOrNull { File(it, "indexes/$mcVersion.json").isFile }
            ?: throw GradleException("No $mcVersion assets found. Run the launcher's --prepare once, or start $mcVersion in the official launcher.")
        File(out, "assets.txt").writeText(assets.path)
    }
}

val runClient by tasks.registering(JavaExec::class) {
    group = "minecraft"
    description = "Runs $mcVersion with the client, offline. -Poptifine=<jar> adds OptiFine; -Pautotest=true makes a world, screenshots it and quits."
    dependsOn(prepareRun, clientJar)
    javaLauncher.set(java8)
    mainClass.set("net.minecraft.launchwrapper.Launch")
    workingDir = runDir.asFile
    doFirst { runDir.asFile.mkdirs() }
    val optifine = providers.gradleProperty("optifine")
    // Read lazily: Gradle looks at the classpath before prepareRun has written classpath.txt.
    classpath = files(provider { runFiles.get().file("classpath.txt").asFile.let { if (it.isFile) it.readLines() else emptyList() } }) +
        files(clientJar.flatMap { it.archiveFile }) +
        files(optifine.map { listOf(it) }.orElse(emptyList())) +
        files(vanillaJar)
    jvmArgs(
        "-Xms512M", "-Xmx2G",
        "-XX:+UnlockExperimentalVMOptions", "-XX:+UseG1GC", "-XX:G1NewSizePercent=20",
        "-XX:G1ReservePercent=20", "-XX:MaxGCPauseMillis=50", "-XX:G1HeapRegionSize=32M",
    )
    val autotest = providers.gradleProperty("autotest").map { it.toBoolean() }.orElse(false)
    // Same logging config the launcher uses (Log4Shell filter, session token line dropped).
    val logConfig = layout.projectDirectory.file("../launcher/assets/log4j2-lean.xml").asFile
    jvmArgumentProviders.add(CommandLineArgumentProvider {
        listOf("-Djava.library.path=${runFiles.get().dir("natives").asFile}", "-Dlog4j.configurationFile=${logConfig.canonicalPath}") +
            (if (autotest.get()) listOf("-Dlean.dev.autotest=true") else emptyList())
    })
    val user = "Dev"
    val uuid = UUID.nameUUIDFromBytes("OfflinePlayer:$user".toByteArray()).toString().replace("-", "")
    argumentProviders.add(CommandLineArgumentProvider {
        listOf(
            "--username", user, "--version", mcVersion, "--gameDir", runDir.asFile.path,
            "--assetsDir", runFiles.get().file("assets.txt").asFile.readText().trim(), "--assetIndex", mcVersion,
            "--uuid", uuid, "--accessToken", "0", "--userProperties", "{}", "--userType", "legacy",
            "--tweakClass", "lean.client.launch.LeanTweaker",
        )
    })
}
