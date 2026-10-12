#!/usr/bin/env python3
"""Small deterministic project generator; no XcodeGen/Ruby dependency."""
from pathlib import Path
import hashlib
root=Path(__file__).resolve().parents[1]/'ios'
def uid(value): return hashlib.sha256(value.encode()).hexdigest()[:24].upper()
files=sorted((root/'Jianxin').glob('*.swift'))
objects=[]
def add(key,content): objects.append(f'{uid(key)} = {{ {content} }};')
for file in files:
    name=file.name
    add('ref:'+name,f'isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = "{name}"; sourceTree = "<group>";')
    add('build:'+name,f'isa = PBXBuildFile; fileRef = {uid("ref:"+name)};')
add('assetref','isa = PBXFileReference; lastKnownFileType = folder.assetcatalog; path = Assets.xcassets; sourceTree = \"<group>\";')
add('assetbuild',f'isa = PBXBuildFile; fileRef = {uid("assetref")};')
add('product','isa = PBXFileReference; explicitFileType = wrapper.application; path = Jianxin.app; sourceTree = BUILT_PRODUCTS_DIR;')
add('sourcegroup','isa = PBXGroup; path = Jianxin; sourceTree = "<group>"; children = ('+','.join([uid('ref:'+f.name) for f in files]+[uid('assetref')])+');')
add('products',f'isa = PBXGroup; name = Products; sourceTree = "<group>"; children = ({uid("product")});')
add('rootgroup',f'isa = PBXGroup; sourceTree = "<group>"; children = ({uid("sourcegroup")},{uid("products")});')
add('sources','isa = PBXSourcesBuildPhase; buildActionMask = 2147483647; files = ('+','.join(uid('build:'+f.name) for f in files)+'); runOnlyForDeploymentPostprocessing = 0;')
add('frameworks','isa = PBXFrameworksBuildPhase; buildActionMask = 2147483647; files = (); runOnlyForDeploymentPostprocessing = 0;')
add('resources',f'isa = PBXResourcesBuildPhase; buildActionMask = 2147483647; files = ({uid("assetbuild")}); runOnlyForDeploymentPostprocessing = 0;')
for mode in ['Debug','Release']:
    flags='SWIFT_ACTIVE_COMPILATION_CONDITIONS = DEBUG; SWIFT_OPTIMIZATION_LEVEL = "-Onone";' if mode=='Debug' else 'SWIFT_OPTIMIZATION_LEVEL = "-Osize"; DEAD_CODE_STRIPPING = YES; STRIP_INSTALLED_PRODUCT = YES;'
    add('project:'+mode,f'isa = XCBuildConfiguration; name = {mode}; buildSettings = {{ SDKROOT = iphoneos; IPHONEOS_DEPLOYMENT_TARGET = 17.0; CLANG_ENABLE_MODULES = YES; }};')
    add('target:'+mode,f'''isa = XCBuildConfiguration; name = {mode}; buildSettings = {{
    ASSETCATALOG_COMPILER_APPICON_NAME = AppIcon; PRODUCT_BUNDLE_IDENTIFIER = com.jianxin.mail; PRODUCT_NAME = Jianxin; SWIFT_VERSION = 5.0; TARGETED_DEVICE_FAMILY = "1,2";
    GENERATE_INFOPLIST_FILE = YES; INFOPLIST_KEY_CFBundleDisplayName = "简信"; INFOPLIST_KEY_UILaunchScreen_Generation = YES;
    INFOPLIST_KEY_UIApplicationSceneManifest_Generation = YES; INFOPLIST_KEY_UIApplicationSupportsIndirectInputEvents = YES;
    INFOPLIST_KEY_UISupportedInterfaceOrientations = "UIInterfaceOrientationPortrait UIInterfaceOrientationPortraitUpsideDown UIInterfaceOrientationLandscapeLeft UIInterfaceOrientationLandscapeRight";
    CURRENT_PROJECT_VERSION = 2; MARKETING_VERSION = 1.0.0; CODE_SIGN_STYLE = Automatic;
    SUPPORTED_PLATFORMS = "iphoneos iphonesimulator"; {flags}
    }};''')
for key in ['project','target']:
    add('configs:'+key,f'isa = XCConfigurationList; buildConfigurations = ({uid(key+":Debug")},{uid(key+":Release")}); defaultConfigurationIsVisible = 0; defaultConfigurationName = Release;')
add('target',f'isa = PBXNativeTarget; name = Jianxin; productName = Jianxin; productType = "com.apple.product-type.application"; productReference = {uid("product")}; buildConfigurationList = {uid("configs:target")}; buildPhases = ({uid("sources")},{uid("frameworks")},{uid("resources")}); buildRules = (); dependencies = ();')
add('project',f'isa = PBXProject; attributes = {{ LastUpgradeCheck = 1620; }}; buildConfigurationList = {uid("configs:project")}; compatibilityVersion = "Xcode 14.0"; developmentRegion = "zh-Hans"; knownRegions = ("zh-Hans",en,Base); mainGroup = {uid("rootgroup")}; productRefGroup = {uid("products")}; projectDirPath = ""; projectRoot = ""; targets = ({uid("target")});')
(root/'Jianxin.xcodeproj/project.pbxproj').write_text('// !$*UTF8*$!\n{ archiveVersion = 1; classes = {}; objectVersion = 56; objects = {\n'+'\n'.join(objects)+f'\n}}; rootObject = {uid("project")}; }}\n')
(root/'Jianxin.xcodeproj/xcshareddata/xcschemes/Jianxin.xcscheme').write_text(f'''<?xml version="1.0" encoding="UTF-8"?>
<Scheme LastUpgradeVersion="1620" version="1.3"><BuildAction parallelizeBuildables="YES" buildImplicitDependencies="YES"><BuildActionEntries><BuildActionEntry buildForTesting="YES" buildForRunning="YES" buildForProfiling="YES" buildForArchiving="YES" buildForAnalyzing="YES"><BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="{uid('target')}" BuildableName="Jianxin.app" BlueprintName="Jianxin" ReferencedContainer="container:Jianxin.xcodeproj"/></BuildActionEntry></BuildActionEntries></BuildAction><LaunchAction buildConfiguration="Debug" useCustomWorkingDirectory="NO" ignoresPersistentStateOnLaunch="NO" debugDocumentVersioning="YES" allowLocationSimulation="YES"><BuildableProductRunnable runnableDebuggingMode="0"><BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="{uid('target')}" BuildableName="Jianxin.app" BlueprintName="Jianxin" ReferencedContainer="container:Jianxin.xcodeproj"/></BuildableProductRunnable></LaunchAction><ArchiveAction buildConfiguration="Release" revealArchiveInOrganizer="YES"/></Scheme>''')
