#![feature(const_trait_impl)]
// TODO: think about colors. Right now Color means sRGBA8
// maybe have f32 oklab colors internally and use .srgba8() to convert?
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Color{
    r:u8, g:u8, b:u8, a:u8
}
impl Color{
    pub const CLEAR:Color = Color{r:0x00, g:0x00, b:0x00, a:0x00};
    pub const WHITE:Color = Color{r:0xFF, g:0xFF, b:0xFF, a:0xFF};
    pub const BLACK:Color = Color{r:0x00, g:0x00, b:0x00, a:0xFF};
    pub fn srgb8(r:u8, g:u8, b:u8, a:u8) -> Self{ Self{r,g,b,a} }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Vertex{
    pub x:i16,
    pub y:i16,
    pub u:u16,
    pub v:u16,
    pub color: Color
}

pub fn gen_quad(x: i16, y: i16, w: i16, h: i16, u:u16, v:u16, color: Color) -> [Vertex;4] {
    assert!(w > 0);
    assert!(h > 0);
    let w_ = w as u16;
    let h_ = h as u16;
    [
        Vertex{x:x+0,  y:y+0, u:u+0,  v:v+0,  color}, // top left
        Vertex{x:x+0,  y:y+h, u:u+0,  v:v+h_, color}, // bottom left
        Vertex{x:x+w,  y:y+0, u:u+w_, v:v+0,  color}, // top right
        Vertex{x:x+w,  y:y+h, u:u+w_, v:v+h_, color}, // bottom right
    ]
}

pub fn gen_rect(x:i16, y:i16, w:i16, h:i16, color: Color) -> [Vertex;4] {
    assert!(w > 0);
    assert!(h > 0);
    [
        Vertex{x:x+0,  y:y+0, u:0xFFFF, v:0xFFFF, color}, // top left
        Vertex{x:x+0,  y:y+h, u:0xFFFF, v:0xFFFF, color}, // bottom left
        Vertex{x:x+w,  y:y+0, u:0xFFFF, v:0xFFFF, color}, // top right
        Vertex{x:x+w,  y:y+h, u:0xFFFF, v:0xFFFF, color}, // bottom right
    ]
}

pub fn div_round(a:i32, b:i32) -> i32 { (a+(b/2))/b }

use core::ops::*;

// Freetype uses this fixed point format a lot, and it's a good for pixel calculations.
#[allow(non_camel_case_types)]
#[repr(transparent)]
#[derive(Clone,Copy)]
pub struct i32q6(pub i32);
impl i32q6 {
    // easy conversion FROM i32q6 (rounds nearest, ties down)
    // TODO: how to deal with overflow? (to i32 is always safe)
    pub const fn i32(self) -> i32 { (self.0+32)>>6 } 
    pub const fn u32(self) -> u32 { ((self.0 as u32)+32)>>6 }
    pub const fn i16(self) -> i16 { self.i32() as i16 }
    pub const fn u16(self) -> u16 { self.u32() as u16 }

    pub fn f32(self) -> f32 { (self.0 as f32)*(1.0/64.0) }
}

impl const Add for i32q6 { type Output=Self; fn add(self, rhs: Self) -> Self { Self(self.0+rhs.0) } }
impl const Sub for i32q6 { type Output=Self; fn sub(self, rhs: Self) -> Self { Self(self.0-rhs.0) } }
impl const AddAssign for i32q6 { fn add_assign(&mut self, rhs: Self) { self.0 += rhs.0;  } }
impl const SubAssign for i32q6 { fn sub_assign(&mut self, rhs: Self) { self.0 -= rhs.0;  } }

// easy conversion functions TO i32q6
pub trait I32q6 { fn q6(self) -> i32q6; }
impl const I32q6 for i32 { fn q6(self) -> i32q6 { i32q6(self<<6) } }
impl const I32q6 for u32 { fn q6(self) -> i32q6 { i32q6((self as i32)<<6) } }

impl I32q6 for f32 {
    fn q6(self) -> i32q6 { 
        // f32 .round() ties to zero, while i32q6 ties down
        if self < 0.0 {
            i32q6(-((self*-64.0).round()) as i32)
        } else {
            i32q6((self*64.0).round() as i32)
        }
    }
}

impl std::fmt::Display for i32q6 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.precision() == None {
            write!(f,"{:.3}", self.f32())
        }else{
            self.f32().fmt(f)
        }
    }
}

pub trait Dot<T>{ fn dot(&self, rhs:Self) -> T; }

#[allow(non_camel_case_types)]
#[derive(Clone,Copy)]
pub struct vec2<T>(pub T,pub T) where T:Clone+Copy;
impl<T> Add for vec2<T> where T:Clone+Copy+Add<Output=T> { type Output=Self; fn add(self, rhs: Self) -> Self { Self(self.0+rhs.0, self.1+rhs.1) } }
impl<T> Sub for vec2<T> where T:Clone+Copy+Sub<Output=T> { type Output=Self; fn sub(self, rhs: Self) -> Self { Self(self.0-rhs.0, self.1-rhs.1) } }
impl<T> Mul<T> for vec2<T> where T:Clone+Copy+Mul<Output=T> { type Output=Self; fn mul(self, rhs: T) -> Self { Self(self.0*rhs, self.1*rhs) } }
impl<T> Div<T> for vec2<T> where T:Clone+Copy+Div<Output=T> { type Output=Self; fn div(self, rhs: T) -> Self { Self(self.0/rhs, self.1/rhs) } }
impl<T,U> AddAssign<vec2<U>> for vec2<T> where T:Clone+Copy+AddAssign<U>, U:Clone+Copy { fn add_assign(&mut self, rhs: vec2<U>) { self.0 += rhs.0; self.1 += rhs.1; } }
impl<T,U> SubAssign<vec2<U>> for vec2<T> where T:Clone+Copy+SubAssign<U>, U:Clone+Copy { fn sub_assign(&mut self, rhs: vec2<U>) { self.0 -= rhs.0; self.1 -= rhs.1; } }

impl<T> Dot<T> for vec2<T> where T:Clone+Copy+Mul<T,Output=T>+Add<T,Output=T> {
    fn dot(&self, rhs:vec2<T>) -> T { self.0 * rhs.0 + self.1 * rhs.1 }
}
impl<T> BitXor<vec2<T>> for vec2<T> where T:Clone+Copy+Mul<T,Output=T>+Sub<T,Output=T> {
    type Output = bivec2<T>;
    fn bitxor(self, rhs: vec2<T>) -> Self::Output {
        bivec2(self.0*rhs.1-self.1*rhs.0)
    }
}

impl<T> std::fmt::Display for vec2<T> where T:Clone+Copy+std::fmt::Display {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(")?; self.0.fmt(f)?; f.write_str(",")?; self.1.fmt(f)?; f.write_str(")")
    }
}

#[allow(non_camel_case_types)]
#[derive(Clone,Copy)]
pub struct bivec2<T>(pub T) where T:Clone+Copy;

#[allow(non_camel_case_types)]
#[derive(Clone,Copy)]
pub struct rotor2<T>(pub vec2<T>,pub bivec2<T>) where T:Clone+Copy;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn i32q6() {
        assert_eq!(13,    13.q6().i32());
        assert_eq!(15*64+64/4, 15.25.q6().0);
        assert_eq!(15.25, 15.25.q6().f32());
        assert_eq!(15.5, 15.5.q6().f32());
        assert_eq!(-15.5, -15.5.q6().f32());
    }
}
