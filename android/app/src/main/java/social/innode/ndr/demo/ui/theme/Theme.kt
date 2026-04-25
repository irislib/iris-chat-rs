package social.innode.ndr.demo.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

private val ColorError = androidx.compose.ui.graphics.Color(0xFFF4212E)

private val LightColors =
    lightColorScheme(
        primary = IrisBlack,
        onPrimary = IrisWhite,
        secondary = IrisMutedLight,
        onSecondary = IrisWhite,
        tertiary = Sky500,
        surface = IrisLightSurface,
        onSurface = IrisBlack,
        surfaceVariant = IrisLightSurfaceAlt,
        onSurfaceVariant = IrisMutedLight,
        outline = IrisLightBorder,
        background = IrisLightBackground,
        onBackground = IrisBlack,
        error = ColorError,
    )

private val DarkColors =
    darkColorScheme(
        primary = IrisPurple,
        onPrimary = IrisWhite,
        secondary = IrisMutedDark,
        onSecondary = IrisBlack,
        tertiary = IrisAccent,
        surface = IrisNightSurface,
        onSurface = IrisWhite,
        surfaceVariant = IrisNightSurfaceAlt,
        onSurfaceVariant = IrisMutedDark,
        outline = IrisNightBorder,
        background = IrisNightBackground,
        onBackground = IrisWhite,
        error = ColorError,
    )

private val IrisTypography =
    Typography(
        headlineLarge =
            TextStyle(
                fontWeight = FontWeight.Bold,
                fontSize = 32.sp,
                lineHeight = 36.sp,
            ),
        headlineMedium =
            TextStyle(
                fontWeight = FontWeight.Bold,
                fontSize = 28.sp,
                lineHeight = 32.sp,
            ),
        headlineSmall =
            TextStyle(
                fontWeight = FontWeight.SemiBold,
                fontSize = 22.sp,
                lineHeight = 28.sp,
            ),
        titleLarge =
            TextStyle(
                fontWeight = FontWeight.SemiBold,
                fontSize = 20.sp,
                lineHeight = 24.sp,
            ),
        titleMedium =
            TextStyle(
                fontWeight = FontWeight.SemiBold,
                fontSize = 16.sp,
                lineHeight = 22.sp,
            ),
        titleSmall =
            TextStyle(
                fontWeight = FontWeight.SemiBold,
                fontSize = 14.sp,
                lineHeight = 18.sp,
            ),
        bodyLarge =
            TextStyle(
                fontWeight = FontWeight.Normal,
                fontSize = 16.sp,
                lineHeight = 22.sp,
            ),
        bodyMedium =
            TextStyle(
                fontWeight = FontWeight.Normal,
                fontSize = 14.sp,
                lineHeight = 20.sp,
            ),
        bodySmall =
            TextStyle(
                fontWeight = FontWeight.Normal,
                fontSize = 12.sp,
                lineHeight = 16.sp,
            ),
        labelLarge =
            TextStyle(
                fontWeight = FontWeight.SemiBold,
                fontSize = 14.sp,
                lineHeight = 18.sp,
            ),
        labelMedium =
            TextStyle(
                fontWeight = FontWeight.Medium,
                fontSize = 12.sp,
                lineHeight = 16.sp,
            ),
        labelSmall =
            TextStyle(
                fontWeight = FontWeight.Medium,
                fontSize = 11.sp,
                lineHeight = 14.sp,
            ),
    )

private val LocalIrisPalette =
    staticCompositionLocalOf<IrisPalette> {
        error("IrisPalette not provided")
    }

@Immutable
data class IrisPalette(
    val panel: androidx.compose.ui.graphics.Color,
    val panelAlt: androidx.compose.ui.graphics.Color,
    val border: androidx.compose.ui.graphics.Color,
    val toolbar: androidx.compose.ui.graphics.Color,
    val bubbleMine: androidx.compose.ui.graphics.Color,
    val bubbleTheirs: androidx.compose.ui.graphics.Color,
    val accent: androidx.compose.ui.graphics.Color,
    val accentAlt: androidx.compose.ui.graphics.Color,
    val muted: androidx.compose.ui.graphics.Color,
)

object IrisTheme {
    val palette: IrisPalette
        @Composable get() = LocalIrisPalette.current
}

@Composable
fun IrisChatTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    val palette =
        if (darkTheme) {
            IrisPalette(
                panel = IrisNightSurface,
                panelAlt = IrisNightSurfaceAlt,
                border = IrisNightBorder,
                toolbar = IrisNightToolbar,
                bubbleMine = IrisNightBubbleMine,
                bubbleTheirs = IrisNightBubbleTheirs,
                accent = IrisPurple,
                accentAlt = IrisAccent,
                muted = IrisMutedDark,
            )
        } else {
            IrisPalette(
                panel = IrisLightSurface,
                panelAlt = IrisLightSurfaceAlt,
                border = IrisLightBorder,
                toolbar = IrisLightToolbar,
                bubbleMine = IrisLightBubbleMine,
                bubbleTheirs = IrisLightBubbleTheirs,
                accent = IrisPurple,
                accentAlt = IrisAccent,
                muted = IrisMutedLight,
            )
        }

    CompositionLocalProvider(LocalIrisPalette provides palette) {
        MaterialTheme(
            colorScheme = if (darkTheme) DarkColors else LightColors,
            typography = IrisTypography,
            content = content,
        )
    }
}
