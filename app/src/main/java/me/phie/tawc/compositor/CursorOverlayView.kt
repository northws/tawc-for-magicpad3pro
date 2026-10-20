// SPDX-License-Identifier: MIT

package me.phie.tawc.compositor

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.view.View

/**
 * Draws the pointer sprite while the host view holds a relative pointer
 * capture.
 *
 * Without a capture Android draws the cursor itself from the `PointerIcon`
 * the compositor pushes. In capture mode it hides the sprite and reports
 * relative motion instead, so the host has to draw it. The compositor still
 * pushes the client's cursor — a bitmap for `wl_pointer.set_cursor`, a shape
 * name for `wp_cursor_shape_v1` — and this view renders the bitmap when
 * there is one and a small built-in glyph otherwise.
 */
class CursorOverlayView(context: Context) : View(context) {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val glyph = Path()

    private var sprite: Bitmap? = null
    private var hotspotX = 0f
    private var hotspotY = 0f
    private var shape = "default"
    private var pointerX = 0f
    private var pointerY = 0f

    init {
        isClickable = false
        isFocusable = false
        isFocusableInTouchMode = false
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        visibility = View.GONE
    }

    /** Client sprite from `wl_pointer.set_cursor`; null clears it. */
    fun setClientBitmap(bitmap: Bitmap?, hotspotX: Float, hotspotY: Float) {
        sprite = bitmap
        this.hotspotX = hotspotX
        this.hotspotY = hotspotY
        if (visibility == View.VISIBLE) invalidate()
    }

    /** `wp_cursor_shape_v1` / CSS name. Replaces any client sprite. */
    fun setShape(shape: String) {
        this.shape = shape
        sprite = null
        if (visibility == View.VISIBLE) invalidate()
    }

    fun moveTo(x: Float, y: Float) {
        pointerX = x
        pointerY = y
        if (visibility == View.VISIBLE) invalidate()
    }

    fun show() {
        if (visibility != View.VISIBLE) visibility = View.VISIBLE
        invalidate()
    }

    fun hide() {
        if (visibility != GONE) visibility = View.GONE
    }

    override fun onDraw(canvas: Canvas) {
        val bitmap = sprite
        if (bitmap != null) {
            paint.reset()
            paint.isAntiAlias = true
            canvas.drawBitmap(bitmap, pointerX - hotspotX, pointerY - hotspotY, paint)
            return
        }
        if (shape.isEmpty() || shape == "none") return

        val unit = resources.displayMetrics.density
        glyph.reset()
        var stroked = false
        when (shape) {
            "text" -> {
                // I-beam: stem plus the two serifs.
                glyph.moveTo(5.5f * unit, 0.5f * unit)
                glyph.lineTo(5.5f * unit, 15.5f * unit)
                glyph.moveTo(2f * unit, 0.5f * unit)
                glyph.lineTo(9f * unit, 0.5f * unit)
                glyph.moveTo(2f * unit, 15.5f * unit)
                glyph.lineTo(9f * unit, 15.5f * unit)
                stroked = true
            }
            "crosshair" -> {
                glyph.moveTo(5.5f * unit, 0.5f * unit)
                glyph.lineTo(5.5f * unit, 15.5f * unit)
                glyph.moveTo(0.5f * unit, 8f * unit)
                glyph.lineTo(10.5f * unit, 8f * unit)
                stroked = true
            }
            else -> {
                // Arrow, hotspot at (0, 0).
                glyph.moveTo(0f, 0f)
                glyph.lineTo(0f, 16.5f * unit)
                glyph.lineTo(4.3f * unit, 12.4f * unit)
                glyph.lineTo(6.9f * unit, 17.6f * unit)
                glyph.lineTo(9.2f * unit, 16.5f * unit)
                glyph.lineTo(6.6f * unit, 11.5f * unit)
                glyph.lineTo(12f * unit, 11.5f * unit)
                glyph.close()
            }
        }
        glyph.offset(pointerX, pointerY)

        paint.reset()
        paint.isAntiAlias = true
        if (stroked) {
            paint.style = Paint.Style.STROKE
            paint.strokeCap = Paint.Cap.ROUND
            paint.strokeWidth = 3f * unit
            paint.color = Color.BLACK
            canvas.drawPath(glyph, paint)
            paint.strokeWidth = 1.4f * unit
            paint.color = Color.WHITE
            canvas.drawPath(glyph, paint)
        } else {
            paint.style = Paint.Style.FILL
            paint.color = Color.WHITE
            paint.setShadowLayer(1.5f * unit, 0f, 0f, Color.BLACK)
            canvas.drawPath(glyph, paint)
            paint.clearShadowLayer()
            paint.style = Paint.Style.STROKE
            paint.strokeWidth = 1f * unit
            paint.color = Color.BLACK
            canvas.drawPath(glyph, paint)
        }
    }
}
