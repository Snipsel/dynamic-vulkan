#![feature(const_trait_impl, const_fn_floating_point_arithmetic)]
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
    pub const fn srgb8(r:u8, g:u8, b:u8, a:u8) -> Self{ Self{r,g,b,a} }
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

pub fn gen_rect(position:Vec2<f32>, extent:Vec2<f32>, color: Color) -> [Vertex;4] {
    let x = position.x.round() as i16;
    let y = position.y.round() as i16;
    let w = extent.x.round()   as i16;
    let h = extent.y.round()   as i16;
    assert!(w > 0);
    assert!(h > 0);
    [
        Vertex{x:x+0,  y:y+0, u:0xFFFF, v:0xFFFF, color}, // top left
        Vertex{x:x+0,  y:y+h, u:0xFFFF, v:0xFFFF, color}, // bottom left
        Vertex{x:x+w,  y:y+0, u:0xFFFF, v:0xFFFF, color}, // top right
        Vertex{x:x+w,  y:y+h, u:0xFFFF, v:0xFFFF, color}, // bottom right
    ]
}

pub const fn div_round(a:i32, b:i32) -> i32 { (a+(b/2))/b }

use core::ops::*;

/*
// Freetype uses this fixed point format a lot, and it's a good for pixel calculations.
// type conversions become unergonomic. keep out of public interface
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
*/

// make vec2() ergonomic to use by automatically coercing
pub trait CoerceToF32 { fn as_f32(self) -> f32; }
impl const CoerceToF32 for f32   { fn as_f32(self) -> f32 { self } }
impl const CoerceToF32 for f64   { fn as_f32(self) -> f32 { self as f32 } }
impl const CoerceToF32 for u8    { fn as_f32(self) -> f32 { self as f32 } }
impl const CoerceToF32 for i8    { fn as_f32(self) -> f32 { self as f32 } }
impl const CoerceToF32 for i16   { fn as_f32(self) -> f32 { self as f32 } }
impl const CoerceToF32 for u16   { fn as_f32(self) -> f32 { self as f32 } }
impl const CoerceToF32 for i32   { fn as_f32(self) -> f32 { self as f32 } }
impl const CoerceToF32 for u32   { fn as_f32(self) -> f32 { self as f32 } }
impl const CoerceToF32 for i64   { fn as_f32(self) -> f32 { self as f32 } }
impl const CoerceToF32 for u64   { fn as_f32(self) -> f32 { self as f32 } }
impl const CoerceToF32 for i128  { fn as_f32(self) -> f32 { self as f32 } }
impl const CoerceToF32 for u128  { fn as_f32(self) -> f32 { self as f32 } }
impl const CoerceToF32 for isize { fn as_f32(self) -> f32 { self as f32 } }
impl const CoerceToF32 for usize { fn as_f32(self) -> f32 { self as f32 } }

pub fn vec2<X,Y>(x:X,y:Y) -> Vec2<f32>
    where X:Clone+Copy+CoerceToF32,
          Y:Clone+Copy+CoerceToF32 {
    Vec2{ x:x.as_f32(), y:y.as_f32() }
}

pub fn vec2t<T,X,Y>(x:X,y:Y) -> Vec2<T>
    where X:Clone+Copy+Into<T>,
          Y:Clone+Copy+Into<T>,
          T:Clone+Copy{
    Vec2{ x:x.into(), y:y.into() }
}

#[derive(Clone,Copy)]
pub struct Vec2<T> where T:Clone+Copy {
    pub x: T, 
    pub y: T,
}

impl<T> Vec2<T> where T:Clone+Copy {
    pub fn map<R>(&self, f:impl Fn(T)->R)->Vec2<R> where R:Clone+Copy {
        Vec2{ x: f(self.x), y: f(self.y) }
    }
    pub fn map2<R,U>(&self, rhs: Vec2<U>, f:impl Fn(T,U)->R)->Vec2<R> where U:Clone+Copy, R:Clone+Copy {
        Vec2{ x: f(self.x, rhs.x), y: f(self.y, rhs.y) }
    }
}

impl<T>   const Add for Vec2<T> where T:Clone+Copy+Add<Output=T> { type Output=Self; fn add(self, rhs: Self) -> Self { self.map2(rhs,|a,b|a+b) } }
impl<T>   const Sub for Vec2<T> where T:Clone+Copy+Sub<Output=T> { type Output=Self; fn sub(self, rhs: Self) -> Self { self.map2(rhs,|a,b|a-b) } }
impl<T>   const Mul<T> for Vec2<T> where T:Clone+Copy+Mul<Output=T> { type Output=Self; fn mul(self, rhs: T) -> Self { vec2t(self.x*rhs, self.y*rhs) } }
impl<T>   const Div<T> for Vec2<T> where T:Clone+Copy+Div<Output=T> { type Output=Self; fn div(self, rhs: T) -> Self { vec2t(self.x/rhs, self.y/rhs) } }
impl<T,U> const AddAssign<Vec2<U>> for Vec2<T> where T:Clone+Copy+AddAssign<U>, U:Clone+Copy { fn add_assign(&mut self, rhs: Vec2<U>) { self.x += rhs.x; self.y += rhs.y; } }
impl<T,U> const SubAssign<Vec2<U>> for Vec2<T> where T:Clone+Copy+SubAssign<U>, U:Clone+Copy { fn sub_assign(&mut self, rhs: Vec2<U>) { self.x -= rhs.x; self.y -= rhs.y; } }

pub trait InnerProduct<T>{ fn dot(&self, rhs:Self) -> T; }
impl<T> InnerProduct<T> for Vec2<T> where T:Clone+Copy+Mul<T,Output=T>+Add<T,Output=T> {
    fn dot(&self, rhs:Vec2<T>) -> T { self.x * rhs.x + self.y * rhs.y }
}
impl<T> BitXor<Vec2<T>> for Vec2<T> where T:Clone+Copy+Mul<T,Output=T>+Sub<T,Output=T> {
    type Output = BiVec2<T>;
    fn bitxor(self, rhs: Vec2<T>) -> Self::Output {
        BiVec2{xy:self.x*rhs.y-self.y*rhs.x}
    }
}

impl<T> std::fmt::Display for Vec2<T> where T:Clone+Copy+std::fmt::Display {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(")?; self.x.fmt(f)?; f.write_str(",")?; self.y.fmt(f)?; f.write_str(")")
    }
}

#[derive(Clone,Copy)]
pub struct BiVec2<T> where T:Clone+Copy {pub xy: T}

#[derive(Clone,Copy)]
pub struct Rotor2<T>(pub Vec2<T>,pub BiVec2<T>) where T:Clone+Copy;

#[cfg(test)]
mod tests {
    use super::*;
    //#[test]
    //fn i32q6() {
    //    assert_eq!(13,    13.q6().i32());
    //    assert_eq!(15*64+64/4, 15.25.q6().0);
    //    assert_eq!(15.25, 15.25.q6().f32());
    //    assert_eq!(15.5, 15.5.q6().f32());
    //    assert_eq!(-15.5, -15.5.q6().f32());
    //}
}
