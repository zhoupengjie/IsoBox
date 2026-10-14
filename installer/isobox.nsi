; IsoBox 安装程序。由 installer\build.ps1 调用 makensis 生成，不要直接编译：
;   powershell -ExecutionPolicy Bypass -File installer\build.ps1
;
; 需要的定义（build.ps1 会传入）：
;   VERSION  版本号，例如 0.1.0
;   STAGE    准备好的文件：四个 exe、isobox.toml、notice.txt、7-Zip\7z.exe、7z.dll、License.txt
;   OUTFILE  生成的安装程序
;   ICON     安装程序图标
;   PREVIEW  可选。只用来看界面：普通权限运行，不修改系统
;
; 命令行参数（静默安装 /S 时使用）：
;   /PROTECT=D:\;E:\     要保护的位置，分号分隔；/PROTECT=none 不保护任何盘。不写时自动保护所有内置非系统盘
;   /EXCHANGE=<文件夹>   交换文件夹。不写时用下载文件夹里的 IsoBox
;   /NOINIT              只装程序文件，不建隔离账户
; 卸载程序：/S 静默卸载，默认保留隔离账户的文件；加 /DELETEDATA 则一起删除
;
; 退出码：0 成功；2 没有安装（系统不支持、安装目录里有别人的文件）；3 程序文件装好了，但隔离账户没建好

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma

!ifdef PREVIEW
  RequestExecutionLevel user
  ; 预览版不装文件：不写卸载程序，也不调用建账户等函数
  !pragma warning disable 6010
  !pragma warning disable 6020
!else
  RequestExecutionLevel admin
!endif

!include MUI2.nsh
!include LogicLib.nsh
!include nsDialogs.nsh
!include FileFunc.nsh
!include WordFunc.nsh
!include x64.nsh
!include WinVer.nsh

!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\IsoBox"
!define WEBVIEW2_GUID "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"

Name "IsoBox"
OutFile "${OUTFILE}"
; 安装位置固定：以管理员身份运行的 isobox-setup 只信任这个只有管理员能写的位置。.onInit 里会再设一次，忽略 /D
InstallDir "$PROGRAMFILES64\IsoBox"
BrandingText "IsoBox ${VERSION}"
ShowInstDetails show
ShowUninstDetails show

Var AccountState    ; ready 可以直接用 / missing 没有 / foreign 同名但不是 IsoBox 建的 / nocred 密码不在当前账户 / unknown 没查到
Var SandboxUser
Var DefaultExchange
Var ProtectArgs     ; 传给 isobox-setup install 的 --protect ... 或 --no-protect，空表示用配置里的
Var ExchangeArg     ; --exchange ...，空表示用配置里的
Var NoInit
Var Unit            ; 字符串 u。nsDialogs 的坐标要写成 "$Yu"，NSIS 会把它当成变量名，只能拼接 "$Y$Unit"
Var ExchangeField
Var ProtectDialog
Var InfoConfig      ; installer-info 用的配置：已安装的或安装包里的模板
Var WelcomeText
Var FinishText
Var GuiWasRunning
Var Launched
Var KeepData
Var KeepCheckbox
Var Forced

; ---------- 页面 ----------
!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_UNABORTWARNING

!define MUI_WELCOMEPAGE_TITLE "安装 IsoBox ${VERSION}"
!define MUI_WELCOMEPAGE_TEXT "$WelcomeText"
!insertmacro MUI_PAGE_WELCOME

!define MUI_PAGE_HEADER_TEXT "使用前须知"
!define MUI_PAGE_HEADER_SUBTEXT "IsoBox 能防住什么、防不住什么"
!define MUI_LICENSEPAGE_TEXT_TOP "请读完下面的说明。"
!define MUI_LICENSEPAGE_TEXT_BOTTOM "了解这些限制后，点“我已了解”继续。"
!define MUI_LICENSEPAGE_BUTTON "我已了解(&A)"
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipUnlessNew
!insertmacro MUI_PAGE_LICENSE "${STAGE}\notice.txt"

Page custom ProtectPage ProtectPageLeave

!insertmacro MUI_PAGE_INSTFILES

!define MUI_FINISHPAGE_TITLE "安装完成"
!define MUI_FINISHPAGE_TEXT "$FinishText"
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "打开 IsoBox"
!define MUI_FINISHPAGE_RUN_FUNCTION LaunchGui
!define MUI_FINISHPAGE_NOAUTOCLOSE
!insertmacro MUI_PAGE_FINISH

UninstPage custom un.DataPage un.DataPageLeave
!insertmacro MUI_UNPAGE_INSTFILES
!define MUI_UNFINISHPAGE_NOAUTOCLOSE
!insertmacro MUI_UNPAGE_FINISH

!insertmacro MUI_LANGUAGE "SimpChinese"
; 默认的宋体 9 号在高分屏上发虚，换成 Windows 自带的微软雅黑 UI
SetFont /LANG=${LANG_SIMPCHINESE} "Microsoft YaHei UI" 9

VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=${LANG_SIMPCHINESE} "ProductName" "IsoBox"
VIAddVersionKey /LANG=${LANG_SIMPCHINESE} "FileDescription" "IsoBox 安装程序"
VIAddVersionKey /LANG=${LANG_SIMPCHINESE} "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=${LANG_SIMPCHINESE} "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=${LANG_SIMPCHINESE} "LegalCopyright" "IsoBox"

; ---------- 安装和卸载共用的小工具 ----------

; 去掉字符串末尾的换行。用到 $R9
!macro TrimNewline var
  ${Do}
    StrCpy $R9 ${var} 1 -1
    ${If} $R9 == "$\r"
    ${OrIf} $R9 == "$\n"
      StrCpy ${var} ${var} -1
    ${Else}
      ${ExitDo}
    ${EndIf}
  ${Loop}
!macroend

; 给命令行参数加引号。末尾是反斜杠时（例如 D:\）要再加一个，否则 \" 会被当成转义的引号
Function QuoteArg
  Exch $0
  Push $1
  StrCpy $1 $0 1 -1
  ${If} $1 == "\"
    StrCpy $0 "$0\"
  ${EndIf}
  StrCpy $0 '"$0"'
  Pop $1
  Exch $0
FunctionEnd

; ---------- 安装 ----------

Function .onInit
  SetShellVarContext all
  StrCpy $Unit "u"
  StrCpy $NoInit 0
  StrCpy $GuiWasRunning 0
  StrCpy $Launched 0
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "IsoBox 只支持 64 位 Windows。" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  ${IfNot} ${AtLeastWin10}
    MessageBox MB_ICONSTOP "IsoBox 需要 Windows 10 或 Windows 11。" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
!ifndef PREVIEW
  UserInfo::GetAccountType
  Pop $0
  ${If} $0 != "admin"
    MessageBox MB_ICONSTOP "需要管理员权限才能安装 IsoBox。" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
!endif
  StrCpy $INSTDIR "$PROGRAMFILES64\IsoBox"
  Call CheckForeign
  Call ReadInstallerInfo
  Call ParseCommandLine

  ${Select} $AccountState
    ${Case} "ready"
      StrCpy $WelcomeText "将把 IsoBox 更新到 ${VERSION}。$\r$\n$\r$\n你的软件、库、受保护的盘和设置都会保留，正在运行的隔离软件不受影响。IsoBox 的界面和托盘会先关闭。$\r$\n$\r$\n点“下一步”开始更新。"
    ${Case} "foreign"
      StrCpy $WelcomeText "将安装 IsoBox ${VERSION}。$\r$\n$\r$\n注意：系统里已经有名为 $SandboxUser 的账户，但它不是 IsoBox 创建的。安装程序只装程序文件，不会动这个账户。装好后请在配置文件 isobox.toml 里把 sandbox_user 改成别的名字，再打开 IsoBox 点“立即安装”。"
    ${Case} "nocred"
      StrCpy $WelcomeText "将安装 IsoBox ${VERSION} 的程序文件。$\r$\n$\r$\n注意：隔离账户 $SandboxUser 已经存在，但它的密码不在当前账户的凭据管理器里，可能是用另一个 Windows 账户安装的。安装程序只更新程序文件，请用当初安装 IsoBox 的账户使用它。"
    ${CaseElse}
      StrCpy $WelcomeText "IsoBox 让微信、QQ、网盘这类你不完全信任的软件，以一个独立的 Windows 账户运行：读不到你的文件，窗口照常显示在你的桌面上，麦克风和摄像头正常可用。$\r$\n$\r$\n安装程序会：$\r$\n  · 把 IsoBox 装到 $INSTDIR$\r$\n  · 创建隐藏的标准账户 $SandboxUser 来运行隔离软件$\r$\n  · 禁止这个账户访问你选择的硬盘$\r$\n$\r$\n点“下一步”继续。"
  ${EndSelect}
FunctionEnd

; 安装目录已存在时，里面只能有 IsoBox 自己的东西：安装程序会锁定这个目录的权限，不能影响别人的文件
Function CheckForeign
  ${IfNot} ${FileExists} "$INSTDIR\*.*"
    Return
  ${EndIf}
  StrCpy $R1 ""
  FindFirst $R0 $R2 "$INSTDIR\*"
  ${DoWhile} $R2 != ""
    ${Switch} $R2
      ${Case} "."
      ${Case} ".."
      ${Case} "isobox.exe"
      ${Case} "isobox-setup.exe"
      ${Case} "isobox-launch.exe"
      ${Case} "isobox-gui.exe"
      ${Case} "uninstall.exe"
      ${Case} "isobox.toml"
      ${Case} "isobox.toml.template"
      ${Case} "library.toml"
      ${Case} "apps"
      ${Case} "backup"
      ${Case} "icons"
      ${Case} "7-Zip"
      ${Case} "add-app.ps1"
      ${Case} "remove-app.ps1"
        ${Break}
      ${Default}
        StrCpy $R1 "$R1    $R2$\r$\n"
    ${EndSwitch}
    FindNext $R0 $R2
  ${Loop}
  FindClose $R0
  ${If} $R1 != ""
    MessageBox MB_ICONSTOP "$INSTDIR 里有不属于 IsoBox 的文件：$\r$\n$\r$\n$R1$\r$\n安装程序会修改这个文件夹的权限。为了不影响这些文件，停止安装，请先把它们移走。" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
FunctionEnd

; 运行安装包里的 isobox-setup installer-info，读出账户状态、默认交换文件夹。每个盘的信息留在文件里，保护页再读
Function ReadInstallerInfo
  StrCpy $AccountState "unknown"
  StrCpy $SandboxUser "isobox"
  StrCpy $DefaultExchange ""
  InitPluginsDir
  File "/oname=$PLUGINSDIR\isobox-setup.exe" "${STAGE}\isobox-setup.exe"
  ; 已经装过就用正在生效的配置，否则用安装包里的模板
  ${If} ${FileExists} "$INSTDIR\isobox.toml"
    StrCpy $InfoConfig "$INSTDIR\isobox.toml"
  ${Else}
    File "/oname=$PLUGINSDIR\isobox.toml" "${STAGE}\isobox.toml"
    StrCpy $InfoConfig "$PLUGINSDIR\isobox.toml"
  ${EndIf}
  nsExec::Exec '"$PLUGINSDIR\isobox-setup.exe" --config "$InfoConfig" --log "$PLUGINSDIR\info.txt" --utf16 installer-info'
  Pop $1
  ClearErrors
  FileOpen $2 "$PLUGINSDIR\info.txt" r
  ${If} ${Errors}
    Return
  ${EndIf}
  ${Do}
    ClearErrors
    FileReadUTF16LE $2 $3
    ${If} ${Errors}
      ${ExitDo}
    ${EndIf}
    !insertmacro TrimNewline $3
    StrCpy $4 $3 8
    ${If} $4 == "account="
      StrCpy $AccountState $3 "" 8
    ${EndIf}
    StrCpy $4 $3 5
    ${If} $4 == "user="
      StrCpy $SandboxUser $3 "" 5
    ${EndIf}
    StrCpy $4 $3 9
    ${If} $4 == "exchange="
      StrCpy $DefaultExchange $3 "" 9
    ${EndIf}
  ${Loop}
  FileClose $2
FunctionEnd

Function ParseCommandLine
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/NOINIT" $R1
  ${IfNot} ${Errors}
    StrCpy $NoInit 1
  ${EndIf}
  ClearErrors
  ${GetOptions} $R0 "/EXCHANGE=" $R1
  ${IfNot} ${Errors}
  ${AndIf} $R1 != ""
    Push $R1
    Call QuoteArg
    Pop $R1
    StrCpy $ExchangeArg "--exchange $R1"
  ${EndIf}
  ClearErrors
  ${GetOptions} $R0 "/PROTECT=" $R1
  ${If} ${Errors}
    Return
  ${EndIf}
  ${If} $R1 == "none"
    StrCpy $ProtectArgs "--no-protect"
    Return
  ${EndIf}
  ; 按分号拆开：$R4 是下一个分号的位置
  ${Do}
    ${If} $R1 == ""
      ${ExitDo}
    ${EndIf}
    StrCpy $R4 0
    ${Do}
      StrCpy $R5 $R1 1 $R4
      ${If} $R5 == ";"
      ${OrIf} $R5 == ""
        ${ExitDo}
      ${EndIf}
      IntOp $R4 $R4 + 1
    ${Loop}
    ${If} $R4 > 0
      StrCpy $R2 $R1 $R4
      Push $R2
      Call QuoteArg
      Pop $R2
      StrCpy $ProtectArgs "$ProtectArgs --protect $R2"
    ${EndIf}
    IntOp $R4 $R4 + 1
    StrCpy $R1 $R1 "" $R4
  ${Loop}
FunctionEnd

; 须知和保护设置只在第一次安装时显示
Function SkipUnlessNew
  ${If} $AccountState != "missing"
  ${AndIf} $AccountState != "unknown"
    Abort
  ${EndIf}
  ${If} $NoInit == 1
    Abort
  ${EndIf}
FunctionEnd

Function ProtectPage
  ; 和 SkipUnlessNew 相同的判断。Abort 要直接写在页面函数里才会跳过页面
  ${If} $AccountState != "missing"
  ${AndIf} $AccountState != "unknown"
    Abort
  ${EndIf}
  ${If} $NoInit == 1
    Abort
  ${EndIf}
  !insertmacro MUI_HEADER_TEXT "保护设置" "选择不让隔离软件访问的硬盘，以及和它互相传文件的交换文件夹"
  nsDialogs::Create 1018
  Pop $ProtectDialog
  ${If} $ProtectDialog == error
    Abort
  ${EndIf}

  ; 先数一下能保护的盘，超过 5 个就分两列
  StrCpy $R7 0
  ClearErrors
  FileOpen $R0 "$PLUGINSDIR\info.txt" r
  ${IfNot} ${Errors}
    ${Do}
      ClearErrors
      FileReadUTF16LE $R0 $R1
      ${If} ${Errors}
        ${ExitDo}
      ${EndIf}
      StrCpy $R2 $R1 6     ; drive=
      StrCpy $R3 $R1 1 8   ; drive=D|1|... 里的“能否保护”
      ${If} $R2 == "drive="
      ${AndIf} $R3 == "1"
        IntOp $R7 $R7 + 1
      ${EndIf}
    ${Loop}
    FileClose $R0
  ${EndIf}

  ${If} $R7 > 0
    ${NSD_CreateLabel} 0 0 100% 22u "勾选的盘会加上“禁止隔离账户访问”的规则，修改前自动备份原来的权限，文件多的盘需要几分钟。你的用户目录（桌面、文档、下载等）默认就是隔离的。"
  ${Else}
    ${NSD_CreateLabel} 0 0 100% 22u "这台电脑上没有可以保护的硬盘。你的用户目录（桌面、文档、下载等）默认就是隔离的，隔离软件读不到。"
  ${EndIf}
  Pop $0

  ; 每个盘一行：能保护的是复选框，用户数据里记下“盘符|默认是否选中”；不能保护的汇总成一行说明
  StrCpy $R6 0    ; 第几个复选框
  StrCpy $R8 ""   ; 不能保护的盘
  ClearErrors
  FileOpen $R0 "$PLUGINSDIR\info.txt" r
  ${IfNot} ${Errors}
    ${Do}
      ClearErrors
      FileReadUTF16LE $R0 $R1
      ${If} ${Errors}
        ${ExitDo}
      ${EndIf}
      !insertmacro TrimNewline $R1
      StrCpy $R2 $R1 6
      ${If} $R2 != "drive="
        ${Continue}
      ${EndIf}
      StrCpy $R1 $R1 "" 6      ; D|1|1|文字
      StrCpy $R3 $R1 1         ; 盘符
      StrCpy $R4 $R1 1 2       ; 能否保护
      StrCpy $R5 $R1 1 4       ; 默认是否选中
      StrCpy $R1 $R1 "" 6      ; 显示的文字
      ${If} $R4 == "1"
        ${If} $R7 > 5
          IntOp $1 $R6 % 2
          IntOp $1 $1 * 150
          IntOp $2 $R6 / 2
          StrCpy $3 "145$Unit"
        ${Else}
          StrCpy $1 0
          StrCpy $2 $R6
          StrCpy $3 "100%"
        ${EndIf}
        IntOp $2 $2 * 12
        IntOp $2 $2 + 26
        ${NSD_CreateCheckbox} "$1$Unit" "$2$Unit" "$3" 11u "$R1"
        Pop $0
        nsDialogs::SetUserData $0 "$R3|$R5"
        ${If} $R5 == "1"
          ${NSD_Check} $0
        ${EndIf}
        IntOp $R6 $R6 + 1
      ${Else}
        ${If} $R8 == ""
          StrCpy $R8 "$R1"
        ${Else}
          StrCpy $R8 "$R8、$R1"
        ${EndIf}
      ${EndIf}
    ${Loop}
    FileClose $R0
  ${EndIf}

  ; 不能保护的盘写在复选框下面
  ${If} $R7 > 5
    IntOp $2 $R6 + 1
    IntOp $2 $2 / 2
  ${Else}
    StrCpy $2 $R6
  ${EndIf}
  IntOp $2 $2 * 12
  IntOp $2 $2 + 28
  ${If} $R8 != ""
    ${NSD_CreateLabel} 0 "$2$Unit" 100% 12u "不能保护：$R8"
    Pop $0
  ${EndIf}

  ${NSD_CreateLabel} 0 102u 100% 10u "交换文件夹：唯一你和隔离软件都能读写的地方，用来互相传文件"
  Pop $0
  ${NSD_CreateText} 0 115u -62u 13u "$DefaultExchange"
  Pop $ExchangeField
  ${NSD_CreateBrowseButton} -58u 114u 58u 15u "浏览..."
  Pop $0
  ${NSD_OnClick} $0 OnBrowseExchange

  nsDialogs::Show
FunctionEnd

Function OnBrowseExchange
  ${NSD_GetText} $ExchangeField $0
  nsDialogs::SelectFolderDialog "选择交换文件夹" "$0"
  Pop $0
  ${If} $0 != error
    ${NSD_SetText} $ExchangeField "$0"
  ${EndIf}
FunctionEnd

; 收集选择，然后让 isobox-setup 预演一遍：交换文件夹不合适、盘不能保护等问题在这里就提示，不用等到安装
Function ProtectPageLeave
  StrCpy $R5 ""   ; --protect 列表
  StrCpy $R6 0    ; 和默认选择不同
  StrCpy $R0 0
  ${Do}
    FindWindow $R0 "Button" "" $ProtectDialog $R0  ; 依次找页面上的按钮（复选框也是按钮）
    ${If} $R0 == 0
      ${ExitDo}
    ${EndIf}
    nsDialogs::GetUserData $R0
    Pop $R1
    ${If} $R1 == ""
      ${Continue}
    ${EndIf}
    StrCpy $R2 $R1 1        ; 盘符
    StrCpy $R3 $R1 1 2      ; 默认是否选中
    ${NSD_GetState} $R0 $R4
    ${If} $R4 == ${BST_CHECKED}
      StrCpy $R5 "$R5 --protect $R2:\"
      ${If} $R3 != "1"
        StrCpy $R6 1
      ${EndIf}
    ${ElseIf} $R3 == "1"
      StrCpy $R6 1
    ${EndIf}
  ${Loop}
  ${If} $R6 == 1
    ${If} $R5 == ""
      StrCpy $ProtectArgs "--no-protect"
    ${Else}
      StrCpy $ProtectArgs $R5
    ${EndIf}
  ${Else}
    StrCpy $ProtectArgs ""
  ${EndIf}

  ${NSD_GetText} $ExchangeField $R1
  ${If} $R1 == ""
    MessageBox MB_ICONEXCLAMATION "请填写交换文件夹。"
    Abort
  ${EndIf}
  ${If} $R1 == $DefaultExchange
    StrCpy $ExchangeArg ""
  ${Else}
    Push $R1
    Call QuoteArg
    Pop $R1
    StrCpy $ExchangeArg "--exchange $R1"
  ${EndIf}

  ; 预演：不加 --apply 只检查并打印计划，不修改系统
  nsExec::Exec '"$PLUGINSDIR\isobox-setup.exe" --config "$InfoConfig" --log "$PLUGINSDIR\check.txt" --utf16 install $ProtectArgs $ExchangeArg'
  Pop $0
  ${If} $0 != 0
    Call ReadError
    Pop $1
    MessageBox MB_ICONEXCLAMATION "这样设置不行：$\r$\n$\r$\n$1"
    Abort
  ${EndIf}
FunctionEnd

; 从预演的输出里取出错误信息（Error: 开头的那行和后面几行），放在栈顶
Function ReadError
  StrCpy $R1 ""
  StrCpy $R2 0
  ClearErrors
  FileOpen $R0 "$PLUGINSDIR\check.txt" r
  ${IfNot} ${Errors}
    ${Do}
      ClearErrors
      FileReadUTF16LE $R0 $R3
      ${If} ${Errors}
        ${ExitDo}
      ${EndIf}
      !insertmacro TrimNewline $R3
      StrCpy $R4 $R3 7
      ${If} $R4 == "Error: "
        StrCpy $R2 1
        StrCpy $R3 $R3 "" 7
      ${EndIf}
      ${If} $R2 == 1
      ${AndIf} $R3 != ""
        StrCpy $R1 "$R1$R3$\r$\n"
      ${EndIf}
    ${Loop}
    FileClose $R0
  ${EndIf}
  ${If} $R1 == ""
    StrCpy $R1 "预演没有通过，原因不明。"
  ${EndIf}
  Push $R1
FunctionEnd

Section "IsoBox" SecMain
  SectionIn RO
  SetDetailsPrint both
!ifdef PREVIEW
  DetailPrint "预览版：不修改系统。下面是 isobox-setup 的实时输出："
  nsExec::ExecToLog '"$PLUGINSDIR\isobox-setup.exe" --config "$InfoConfig" --utf16 drives'
  Pop $0
  nsExec::ExecToLog '"$PLUGINSDIR\isobox-setup.exe" --config "$InfoConfig" --utf16 install $ProtectArgs $ExchangeArg'
  Pop $0
  StrCpy $FinishText "预览版到此结束，没有修改系统。"
!else
  DetailPrint "关闭 IsoBox 界面和后台的深浅色同步……"
  Call StopRunning

  CreateDirectory "$INSTDIR"
  ; isobox-setup.exe 以管理员身份运行。普通用户要是能替换它，任何程序都可能借此拿到管理员权限。
  ; 所以和系统的 Program Files 一样：管理员和 SYSTEM 完全控制，普通用户只能读取和运行。
  ; 库的 apps 目录关闭了继承，有自己的权限，不受影响
  DetailPrint "锁定安装目录的权限：只有管理员能修改"
  nsExec::ExecToLog /OEM 'icacls "$INSTDIR" /inheritance:r /grant:r *S-1-5-32-544:(OI)(CI)F *S-1-5-18:(OI)(CI)F *S-1-5-32-545:(OI)(CI)RX /q'
  Pop $0
  ${If} $0 != 0
    SetErrorLevel 2
    Abort "无法设置 $INSTDIR 的权限（icacls 返回 $0）"
  ${EndIf}

  SetOutPath "$INSTDIR"
  SetOverwrite on
  File "${STAGE}\isobox.exe"
  File "${STAGE}\isobox-setup.exe"
  File "${STAGE}\isobox-launch.exe"
  File "${STAGE}\isobox-gui.exe"
  SetOutPath "$INSTDIR\7-Zip"
  File "${STAGE}\7-Zip\7z.exe"
  File "${STAGE}\7-Zip\7z.dll"
  File "${STAGE}\7-Zip\License.txt"
  SetOutPath "$INSTDIR"
  ; 安装目录里的配置才是实际生效的，更新时绝不覆盖
  ${IfNot} ${FileExists} "$INSTDIR\isobox.toml"
    File "${STAGE}\isobox.toml"
  ${EndIf}

  ; 旧版本用 deploy.ps1 部署时留下的脚本和开始菜单项，已被界面取代
  Delete "$INSTDIR\add-app.ps1"
  Delete "$INSTDIR\remove-app.ps1"
  SetShellVarContext current
  Delete "$SMPROGRAMS\IsoBox\IsoBox.lnk"
  Delete "$SMPROGRAMS\IsoBox\IsoBox 添加软件.lnk"
  Delete "$SMPROGRAMS\IsoBox\IsoBox 移除软件.lnk"
  RMDir "$SMPROGRAMS\IsoBox"
  SetShellVarContext all
  ; 和隔离软件的快捷方式（在你自己的开始菜单 IsoBox 分组里）合并显示在同一个分组
  CreateDirectory "$SMPROGRAMS\IsoBox"
  CreateShortcut "$SMPROGRAMS\IsoBox\IsoBox.lnk" "$INSTDIR\isobox-gui.exe" "" "$INSTDIR\isobox-gui.exe" 0 SW_SHOWNORMAL "" "管理隔离软件"

  WriteUninstaller "$INSTDIR\uninstall.exe"
  SetRegView 64
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "IsoBox"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "IsoBox"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\isobox-gui.exe"
  WriteRegStr HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegStr HKLM "${UNINST_KEY}" "URLInfoAbout" "https://github.com/zhoupengjie/IsoBox"
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1
  ; 只算程序文件，不算库里的软件
  ${GetSize} "$INSTDIR" "/S=0K /G=0" $0 $1 $2
  ${GetSize} "$INSTDIR\7-Zip" "/S=0K" $3 $1 $2
  IntOp $0 $0 + $3
  WriteRegDWORD HKLM "${UNINST_KEY}" "EstimatedSize" $0
  SetRegView 32

  Call InitAccount
  Call CheckWebView2
!endif
SectionEnd

; 建隔离账户、系统盘的库，保护选中的盘，建交换文件夹。已经建好的（更新时）跳过
Function InitAccount
  ${If} $AccountState == "ready"
    StrCpy $FinishText "IsoBox 已更新到 ${VERSION}。"
    Return
  ${EndIf}
  ${If} $NoInit == 1
    StrCpy $FinishText "IsoBox 的程序文件已经装好。打开 IsoBox，点“立即安装”创建隔离账户。"
    Return
  ${EndIf}
  ${If} $AccountState == "foreign"
  ${OrIf} $AccountState == "nocred"
    DetailPrint "隔离账户 $SandboxUser 不能直接使用，只更新了程序文件，原因见欢迎页的说明"
    StrCpy $FinishText "IsoBox 的程序文件已经装好，但隔离账户 $SandboxUser 不能直接使用，原因见欢迎页的说明。"
    SetErrorLevel 3
    Return
  ${EndIf}
  DetailPrint ""
  DetailPrint "创建隔离账户、建库、保护选中的盘。备份硬盘的权限可能需要几分钟，请稍候……"
  nsExec::ExecToLog '"$INSTDIR\isobox-setup.exe" --utf16 --non-interactive install --apply $ProtectArgs $ExchangeArg'
  Pop $0
  ${If} $0 == 0
    StrCpy $FinishText "IsoBox 已经装好，隔离账户也已建好。$\r$\n$\r$\n打开 IsoBox，点“添加软件”，选择微信、QQ 等软件的安装包，就能把它们装进隔离账户。"
  ${Else}
    SetErrorLevel 3
    StrCpy $FinishText "IsoBox 的程序文件已经装好，但隔离账户没有建好，原因见上一页的记录。$\r$\n$\r$\n处理后打开 IsoBox，点“立即安装”重试。"
    MessageBox MB_ICONEXCLAMATION "隔离账户没有建好，原因见安装窗口里的记录。$\r$\n$\r$\n程序文件已经装好。处理后打开 IsoBox，点“立即安装”即可重试。" /SD IDOK
  ${EndIf}
FunctionEnd

; 结束界面和主题监视进程，它们占用着要替换的程序文件。隔离软件本身不受影响
!macro StopRunningFunc prefix
  Function ${prefix}StopRunning
    nsExec::ExecToStack 'tasklist /fi "imagename eq isobox-gui.exe" /nh'
    Pop $0
    Pop $1
    ClearErrors
    ${WordFind} "$1" "isobox-gui.exe" "E+1{" $2
    ${IfNot} ${Errors}
      StrCpy $GuiWasRunning 1
    ${EndIf}
    nsExec::Exec 'taskkill /f /im isobox-gui.exe'
    Pop $0
    nsExec::Exec 'taskkill /f /im isobox-launch.exe'
    Pop $0
    Sleep 800
  FunctionEnd
!macroend
!insertmacro StopRunningFunc ""

; IsoBox 的界面用 Microsoft Edge WebView2 显示。Windows 11 自带，Windows 10 多数也已经装了
Function CheckWebView2
  SetRegView 64
  ReadRegStr $0 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\${WEBVIEW2_GUID}" "pv"
  ${If} $0 == ""
  ${OrIf} $0 == "0.0.0.0"
    ReadRegStr $0 HKCU "Software\Microsoft\EdgeUpdate\Clients\${WEBVIEW2_GUID}" "pv"
  ${EndIf}
  SetRegView 32
  ${If} $0 == ""
  ${OrIf} $0 == "0.0.0.0"
    DetailPrint "注意：没有找到 Microsoft Edge WebView2 运行库，IsoBox 的界面需要它"
    MessageBox MB_ICONINFORMATION "IsoBox 的界面需要 Microsoft Edge WebView2 运行库，这台电脑上没有找到。$\r$\n$\r$\n可以从微软网站下载安装：$\r$\nhttps://go.microsoft.com/fwlink/p/?LinkId=2124703$\r$\n$\r$\n隔离软件的启动不受影响。" /SD IDOK
  ${EndIf}
FunctionEnd

; 安装程序以管理员身份运行。交给资源管理器启动，界面就以普通权限运行
Function LaunchGui
  Exec '"$WINDIR\explorer.exe" "$INSTDIR\isobox-gui.exe"'
  StrCpy $Launched 1
FunctionEnd

; 更新前界面（托盘）在运行，装完又没从完成页打开的话，重新打开托盘
Function .onInstSuccess
  ${If} $GuiWasRunning == 1
  ${AndIf} $Launched == 0
    SetShellVarContext current
    ${If} ${FileExists} "$SMSTARTUP\IsoBox.lnk"
      ; 开机启动的快捷方式带着 --tray 参数，只显示托盘
      Exec '"$WINDIR\explorer.exe" "$SMSTARTUP\IsoBox.lnk"'
    ${Else}
      Exec '"$WINDIR\explorer.exe" "$INSTDIR\isobox-gui.exe"'
    ${EndIf}
    SetShellVarContext all
  ${EndIf}
FunctionEnd

; ---------- 卸载 ----------


!insertmacro StopRunningFunc "un."

Function un.onInit
  SetShellVarContext all
  UserInfo::GetAccountType
  Pop $0
  ${If} $0 != "admin"
    MessageBox MB_ICONSTOP "需要管理员权限才能卸载 IsoBox。" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  StrCpy $KeepData 1
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/DELETEDATA" $R1
  ${IfNot} ${Errors}
    StrCpy $KeepData 0
  ${EndIf}
FunctionEnd

Function un.DataPage
  !insertmacro MUI_HEADER_TEXT "卸载 IsoBox" "撤销 IsoBox 对系统的所有修改"
  nsDialogs::Create 1018
  Pop $0
  ${If} $0 == error
    Abort
  ${EndIf}
  ${NSD_CreateLabel} 0 0 100% 44u "卸载会按安全的顺序撤销 IsoBox 对系统的修改：撤销所有盘上的禁止规则和交换文件夹的授权，删除所有库里的隔离软件、隔离账户和它的个人文件夹，最后删除 IsoBox 本身。$\r$\n$\r$\n隔离软件的聊天记录、收到的文件，都在隔离账户的个人文件夹里。"
  Pop $0
  ${NSD_CreateCheckbox} 0 52u 100% 12u "保留隔离账户的文件（包括微信、QQ 的聊天记录），移到交换文件夹"
  Pop $KeepCheckbox
  ${If} $KeepData == 1
    ${NSD_Check} $KeepCheckbox
  ${EndIf}
  ${NSD_CreateLabel} 12u 68u -12u 40u "保留的是隔离账户的文档、桌面、下载、图片、视频、音乐文件夹里的文件，放在交换文件夹的“隔离账户的文件”里。不勾选的话，这些文件会和隔离账户一起删除，无法恢复。$\r$\n$\r$\n交换文件夹本身的文件始终保留。"
  Pop $0
  ; 这一页之后直接开始卸载，下一步按钮改叫“卸载”
  GetDlgItem $0 $HWNDPARENT 1
  SendMessage $0 ${WM_SETTEXT} 0 "STR:卸载(&U)"
  nsDialogs::Show
FunctionEnd

Function un.DataPageLeave
  ${NSD_GetState} $KeepCheckbox $0
  ${If} $0 == ${BST_CHECKED}
    StrCpy $KeepData 1
  ${Else}
    StrCpy $KeepData 0
  ${EndIf}
FunctionEnd

Section "Uninstall"
  SetDetailsPrint both
  StrCpy $Forced 0
  DetailPrint "关闭 IsoBox 界面和后台的深浅色同步……"
  Call un.StopRunning

  ; 先由 isobox-setup 按安全的顺序撤销对系统的修改。它失败时默认停下，保留程序，方便处理后再卸载
  ${If} ${FileExists} "$INSTDIR\isobox-setup.exe"
    ${If} $KeepData == 1
      StrCpy $0 "--keep-data"
    ${Else}
      StrCpy $0 ""
    ${EndIf}
    DetailPrint "撤销 IsoBox 对系统的修改……"
    nsExec::ExecToLog '"$INSTDIR\isobox-setup.exe" --utf16 --non-interactive uninstall --apply $0'
    Pop $1
    ${If} $1 != 0
      MessageBox MB_YESNO|MB_ICONEXCLAMATION|MB_DEFBUTTON2 "有些清理步骤没有完成，原因见窗口里的记录。$\r$\n$\r$\n选“否”：保留 IsoBox，处理后可以再次卸载（推荐）。$\r$\n选“是”：仍然删除 IsoBox 的程序文件。" /SD IDNO IDYES force
      SetErrorLevel 3
      Abort "卸载没有完成，IsoBox 保留在 $INSTDIR"
      force:
      StrCpy $Forced 1
    ${EndIf}
  ${EndIf}

  ; 只删安装程序放进来的文件。系统盘的库也在这个目录里，上一步已经清理过
  Delete "$INSTDIR\isobox.exe"
  Delete "$INSTDIR\isobox-setup.exe"
  Delete "$INSTDIR\isobox-launch.exe"
  Delete "$INSTDIR\isobox-gui.exe"
  Delete "$INSTDIR\add-app.ps1"
  Delete "$INSTDIR\remove-app.ps1"
  Delete "$INSTDIR\isobox.toml"
  Delete "$INSTDIR\isobox.toml.template"
  RMDir /r "$INSTDIR\7-Zip"
  RMDir /r "$INSTDIR\icons"
  ${If} $Forced == 0
    ; 禁止规则都已撤销，修改前的权限备份不再需要
    RMDir /r "$INSTDIR\backup"
  ${ElseIf} ${FileExists} "$INSTDIR\backup\*.*"
    DetailPrint "保留了权限备份 $INSTDIR\backup。重新安装 IsoBox 后，可以在“保护状态”里用它还原"
  ${EndIf}
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  ${If} ${FileExists} "$INSTDIR\*.*"
    DetailPrint "$INSTDIR 里还有其他文件，保留了这个文件夹"
  ${EndIf}

  Delete "$SMPROGRAMS\IsoBox\IsoBox.lnk"
  RMDir "$SMPROGRAMS\IsoBox"
  SetRegView 64
  DeleteRegKey HKLM "${UNINST_KEY}"
  SetRegView 32
SectionEnd
